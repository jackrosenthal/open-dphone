// SPDX-License-Identifier: GPL-2.0-only
/*
 * Broadcom BCMRING (BCM11107 family: BCM11109/11170/11188/11189) "APM"
 * analog audio front end -- ASoC component and DAI driver.
 *
 * DRAFT.  Written from register behavior recovered by static analysis of
 * vendor binaries.  It has been compile tested only; it has never been run
 * on hardware.  Everything marked "UNVERIFIED" is an inference that needs a
 * board to confirm.  "Measured" refers to register dumps of an idle D40, D50,
 * and D60 running the stock firmware (which streams on all four FIFOs all
 * the time); they show what the vendor driver leaves in the registers, not
 * what this driver does.
 *
 * The block contains two full-duplex mono voice channels ("CH0" and "CH1";
 * a third, capture-only channel exists in the IP but the SoCs used here
 * report only two).  Each channel has one ADC with a two-way microphone
 * input selector (A/B) and one DAC feeding two output drivers (A/B).
 * Samples are 16-bit signed mono, moved through four FIFOs by an external
 * DMA controller.
 *
 * FIFO format: the vendor driver moves its buffers of consecutive
 * little-endian 16-bit samples to and from the FIFOs unconverted, as 32-bit
 * words, so one FIFO word holds two samples with the earlier one in bits
 * 15:0.  This driver uses the same 32-bit accesses and therefore the same
 * layout.
 */

#include <linux/bitfield.h>
#include <linux/bits.h>
#include <linux/clk.h>
#include <linux/delay.h>
#include <linux/dmaengine.h>
#include <linux/interrupt.h>
#include <linux/mfd/syscon.h>
#include <linux/module.h>
#include <linux/mod_devicetable.h>
#include <linux/platform_device.h>
#include <linux/regmap.h>
#include <linux/reset.h>
#include <linux/spinlock.h>

#include <sound/dmaengine_pcm.h>
#include <sound/pcm.h>
#include <sound/pcm_params.h>
#include <sound/soc.h>
#include <sound/soc-dapm.h>
#include <sound/tlv.h>

#define APM_NUM_CH		2

/* ---- Register map (offsets from the block base, all 32 bit) ---- */

#define APM_ENABLE		0x000	/* 0xf = everything on; measured: reads 0 */
#define APM_IRQ_STATUS		0x004	/* write 1 to clear */
#define APM_IRQ_MASK		0x008	/* 1 = enabled */
#define  APM_IRQ_MICDET0	BIT(0)
#define  APM_IRQ_MICON0		BIT(1)
#define  APM_IRQ_MICDET1	BIT(2)
#define  APM_IRQ_MICON1		BIT(3)
#define  APM_IRQ_DAC_UFLOW(c)	BIT(4 + (c))
#define  APM_IRQ_ADC_OFLOW(c)	BIT(6 + (c))
#define APM_DMA_CTRL		0x00c
/*
 * Four 7 bit burst sizes: channel 0 capture, channel 0 playback, channel 1
 * capture, channel 1 playback, from bit 0 up.
 */
#define  APM_DMA_BURST		GENMASK(27, 0)
/*
 * DMA request enables.  The vendor code only tells the channels apart (28
 * and 30 are channel 0, 29 and 31 channel 1) and always switches both
 * directions of a channel together.  UNVERIFIED: that 28/29 are capture and
 * 30/31 playback is inferred from the capture-only third channel, whose
 * registers repeat the channel 0 capture positions and whose enable is bit
 * 28.  The driver does not depend on it: it switches all four together.
 * Measured while streaming: the register reads 0xf0810204.
 */
#define  APM_DMA_EN_RX(c)	BIT(28 + (c))
#define  APM_DMA_EN_TX(c)	BIT(30 + (c))
#define  APM_DMA_EN_CH01	GENMASK(31, 28)
#define APM_COEF_DATA		0x010
#define APM_COEF_ADDR		0x014
#define  APM_COEF_STROBE	BIT(8)
#define APM_LOOPBACK(c)		(0x018 + 4 * (c))

/* Per-channel digital path; channel 1 is the same layout 0x30 higher. */
#define APM_CH(c, r)		((r) + 0x30 * (c))
#define APM_FILT_CTRL(c)	APM_CH(c, 0x020)
#define  APM_FILT_ADC_EQ_EN	BIT(27)
#define  APM_FILT_DAC_EQ_EN	BIT(26)
#define  APM_FILT_ADC_EQ_LEN	GENMASK(14, 8)
#define  APM_FILT_DAC_EQ_LEN	GENMASK(6, 0)
#define APM_SIDETONE(c)		APM_CH(c, 0x024)
#define  APM_SIDETONE_EN	BIT(15)
#define APM_DAC_CFG(c)		APM_CH(c, 0x028)
#define  APM_DAC_CFG_HIRATE	GENMASK(28, 27)
#define APM_DAC_GAIN(c)		APM_CH(c, 0x02c)
#define  APM_DAC_GAIN_SHIFT	GENMASK(18, 16)
#define  APM_DAC_GAIN_MANT	GENMASK(15, 0)
#define APM_ADC_GAIN(c)		APM_CH(c, 0x040)
#define  APM_ADC_GAIN_HIRATE	GENMASK(30, 29)
#define  APM_ADC_GAIN_SHIFT	GENMASK(27, 24)
#define  APM_ADC_GAIN_MANT	GENMASK(15, 0)
#define APM_RATE_PHASE(c)	APM_CH(c, 0x044)

/* Channel run gates live in channel 0's filter control register. */
#define  APM_FILT_CH_HOLD(c)	BIT(31 - (c))

#define APM_NCO_DATA		0x080
#define APM_NCO_CTRL		0x084
#define  APM_NCO_LOAD_BASE(c)	BIT(1 + (c))
#define  APM_NCO_LOAD_RATE(c)	BIT(3 + (c))
#define  APM_NCO_LIMIT		GENMASK(11, 8)
#define APM_NCO_BASE_64K	0x53e2d624	/* 64000 * 2^41 / 100 MHz */

/*
 * Written with all ones by the vendor's enable sequence.  UNVERIFIED names:
 * they are not plain storage (measured afterwards: 0x104 = 5, 0x10c = 5 or
 * 0x08000005, 0x114 = 0x8000), so write-one-to-clear status or flush strobes.
 */
#define APM_FIFO_FLUSH_RX(c)	(0x104 + 8 * (c))
#define APM_FIFO_FLUSH_TX	0x114

#define APM_ANA_PWR		0x120
#define  APM_ANA_PWR_MICBIAS0_N	1	/* bit numbers; 1 = bias off */
#define  APM_ANA_PWR_ADC0	3
#define  APM_ANA_PWR_MICBIAS1_N	4
#define  APM_ANA_PWR_ADC1	6
#define  APM_ANA_PWR_ADC_ALL	(GENMASK(6, 5) | GENMASK(3, 2))
#define  APM_ANA_PWR_MICBIAS_ALL_N (BIT(APM_ANA_PWR_MICBIAS0_N) | \
				    BIT(APM_ANA_PWR_MICBIAS1_N))
#define APM_MIC_CTRL(c)		(0x124 + 8 * (c))
#define  APM_MIC_SEL_B		25
#define  APM_MIC_PGA		GENMASK(24, 19)
#define APM_OUT_PWR		0x134
#define  APM_OUT_PWR_DRV_N(c, o) (18 - 2 * (c) + (o))	/* 1 = driver off */
#define  APM_OUT_PWR_DRV_ALL_N	GENMASK(19, 16)
/* Output ramp, one 4 bit group per driver: direction, then (ms / 20) - 1. */
#define  APM_OUT_RAMP_SHIFT(c, o) (8 - 8 * (c) + 4 * (o))
#define  APM_OUT_RAMP_UP(c, o)	BIT(APM_OUT_RAMP_SHIFT(c, o))
#define  APM_OUT_RAMP_TIME(c, o) (7 << (APM_OUT_RAMP_SHIFT(c, o) + 1))
#define  APM_OUT_RAMP_MS	40	/* vendor value */
#define  APM_OUT_RAMP_WAIT_MS	100	/* vendor: 10 jiffies at HZ = 100 */
#define APM_OUT_CTRL		0x138
#define  APM_OUT_ATTEN_SHIFT(c, o) (26 - 6 * (c) + 3 * (o))
#define  APM_OUT_MODE_SHIFT(c, o) (16 - 4 * (c) + 2 * (o))
#define  APM_OUT_MODE_ON	0
#define  APM_OUT_MODE_MUTE	3
#define APM_DAC_PWR		0x13c
#define  APM_DAC_PWR_N(c)	(10 - (c))		/* 1 = DAC off */
#define  APM_DAC_RAMP_EN(c)	BIT(30 - (c))

#define APM_MAX_REGISTER	0x56c

/* Coefficient RAM selectors, OR-ed into APM_COEF_ADDR. */
#define APM_COEF_ADC_EQ(c)	((c) ? 0x00800000 : 0x00010000)
#define APM_COEF_DAC_EQ(c)	((c) ? 0x01000000 : 0x00020000)
#define APM_COEF_SIDETONE(c)	((c) ? 0x08000000 : 0x00100000)
#define APM_EQ_TAPS		128
#define APM_SIDETONE_WORDS	28

/*
 * SoC chip-control ("chipc") bits that gate the block.  UNVERIFIED names;
 * the register reads exactly 0x97 on all three measured phones.
 */
#define CHIPC_AUDIO_ENABLE	0x7c
#define  CHIPC_AUDIO_EN_APM	0x97	/* clocks + channels A and B */

#define APM_FIFO_BURST		4
#define APM_GAIN_DB_MIN		(-50)
#define APM_GAIN_DB_MAX		18
#define APM_GAIN_STEPS		(APM_GAIN_DB_MAX - APM_GAIN_DB_MIN)
#define APM_Q12_UNITY		0x1000

enum apm_gain_kind { APM_GAIN_ADC, APM_GAIN_DAC, APM_GAIN_SIDETONE, APM_GAIN_KINDS };

struct apm_rate {
	unsigned int rate;
	u32 phase;	/* rate * 2^32 / 64000 */
	u16 scale;	/* ADC gain normalization, nco >> 15 */
	u32 nco;	/* rate * 2^41 / 100 MHz */
};

/* Values used by the vendor hardware layer for the rates its driver allows. */
static const struct apm_rate apm_rates[] = {
	{  8000, 0x20000000, 0x14f8, 0x0a7c5ac4 },
	{ 16000, 0x40000000, 0x29f1, 0x14f8b589 },
	{ 22050, 0x58333333, 0x39cd, 0x1ce6c094 },
	{ 24000, 0x60000000, 0x3eea, 0x1f75104d },
	{ 32000, 0x80000000, 0x53e2, 0x29f16b12 },
	{ 40000, 0xa0000000, 0x68db, 0x346dc5d6 },
	{ 44100, 0xb0666666, 0x739b, 0x39cd8128 },
	{ 48000, 0xc0000000, 0x7dd4, 0x3eea209b },
};

static const unsigned int apm_rate_list[] = {
	8000, 16000, 22050, 24000, 32000, 40000, 44100, 48000,
};

static const struct snd_pcm_hw_constraint_list apm_rate_constraint = {
	.count = ARRAY_SIZE(apm_rate_list),
	.list = apm_rate_list,
};

/* Linear Q12 gain for -50 dB .. +18 dB in 1 dB steps (vendor table). */
static const u16 apm_q12_gain[APM_GAIN_STEPS + 1] = {
	0x000d, 0x000f, 0x0010, 0x0012, 0x0015, 0x0017, 0x001a, 0x001d,
	0x0021, 0x0025, 0x0029, 0x002e, 0x0034, 0x003a, 0x0041, 0x0049,
	0x0052, 0x005c, 0x0067, 0x0073, 0x0082, 0x0091, 0x00a3, 0x00b7,
	0x00cd, 0x00e6, 0x0102, 0x0122, 0x0145, 0x016d, 0x019a, 0x01cc,
	0x0204, 0x0243, 0x0289, 0x02d8, 0x0331, 0x0395, 0x0405, 0x0482,
	0x050f, 0x05ad, 0x065f, 0x0726, 0x0805, 0x08ff, 0x0a18, 0x0b54,
	0x0cb6, 0x0e43, 0x1000, 0x11f4, 0x1425, 0x169a, 0x195c, 0x1c74,
	0x1fed, 0x23d2, 0x2831, 0x2d18, 0x3299, 0x38c5, 0x3fb2, 0x4778,
	0x5031, 0x59fa, 0x64f4, 0x7145, 0x7f18,
};

struct apm_priv {
	struct device *dev;
	struct regmap *regmap;
	struct clk *clk;
	struct dma_chan *chan[APM_NUM_CH][2];
	struct snd_dmaengine_dai_dma_data dma_data[APM_NUM_CH][2];
	const struct apm_rate *rate[APM_NUM_CH];
	u8 gain[APM_NUM_CH][APM_GAIN_KINDS];	/* index into apm_q12_gain */
	/*
	 * Protects active, rate, and the register sequences that share the
	 * rate generator data register between the channels.
	 */
	spinlock_t lock;
	unsigned long active;			/* started streams, APM_STREAM() */
	unsigned long open;			/* opened streams, APM_STREAM() */
};

#define APM_STREAM(ch, dir)	BIT(2 * (ch) + (dir))

/* ---- Hardware helpers ---- */

static void apm_coef_write(struct apm_priv *apm, u32 sel, const u32 *coef,
			   unsigned int count)
{
	unsigned int i;

	regmap_write(apm->regmap, APM_COEF_ADDR, 0);
	for (i = 0; i < count; i++) {
		regmap_write(apm->regmap, APM_COEF_DATA, coef[i]);
		regmap_write(apm->regmap, APM_COEF_ADDR, sel | i);
		regmap_write(apm->regmap, APM_COEF_ADDR, sel | i | APM_COEF_STROBE);
		regmap_write(apm->regmap, APM_COEF_ADDR, sel | i);
	}
	regmap_write(apm->regmap, APM_COEF_ADDR, 0);
}

/* Gains are a 16 bit mantissa with a right-justifying shift count. */
static void apm_split_gain(u32 value, u32 *mant, u32 *shift)
{
	*shift = 3;
	while (value > 0x7fff) {
		value >>= 1;
		(*shift)++;
	}
	*mant = value;
}

static void apm_write_adc_gain(struct apm_priv *apm, unsigned int ch)
{
	u32 q12 = apm_q12_gain[apm->gain[ch][APM_GAIN_ADC]];
	u32 mant, shift;

	apm_split_gain((apm->rate[ch]->scale * q12) >> 12, &mant, &shift);
	regmap_update_bits(apm->regmap, APM_ADC_GAIN(ch),
			   APM_ADC_GAIN_SHIFT | APM_ADC_GAIN_MANT,
			   FIELD_PREP(APM_ADC_GAIN_SHIFT, shift) |
			   FIELD_PREP(APM_ADC_GAIN_MANT, mant));
}

static void apm_write_dac_gain(struct apm_priv *apm, unsigned int ch)
{
	u32 q12 = apm_q12_gain[apm->gain[ch][APM_GAIN_DAC]];
	u32 mant, shift;

	apm_split_gain((q12 * 0x7fff) >> 12, &mant, &shift);
	regmap_update_bits(apm->regmap, APM_DAC_GAIN(ch),
			   APM_DAC_GAIN_SHIFT | APM_DAC_GAIN_MANT,
			   FIELD_PREP(APM_DAC_GAIN_SHIFT, shift) |
			   FIELD_PREP(APM_DAC_GAIN_MANT, mant));
}

static void apm_write_sidetone_gain(struct apm_priv *apm, unsigned int ch)
{
	u32 coef[APM_SIDETONE_WORDS] = { };
	unsigned int was_on;

	/*
	 * The sidetone path is a small filter whose coefficient set is mostly
	 * fixed; only word 6 carries the level.  The meaning of the other
	 * words is not known.
	 */
	coef[0] = 0x7ffff;
	coef[5] = 0x66;
	coef[6] = min_t(u32, apm_q12_gain[apm->gain[ch][APM_GAIN_SIDETONE]] << 7,
			0x7ffff);
	coef[24] = 6;
	coef[25] = 0x7ffff;
	coef[27] = 0x66;

	/* The vendor code gates the path off while it is reloaded. */
	regmap_read(apm->regmap, APM_SIDETONE(ch), &was_on);
	regmap_clear_bits(apm->regmap, APM_SIDETONE(ch), APM_SIDETONE_EN);
	apm_coef_write(apm, APM_COEF_SIDETONE(ch), coef, ARRAY_SIZE(coef));
	if (was_on & APM_SIDETONE_EN)
		regmap_set_bits(apm->regmap, APM_SIDETONE(ch), APM_SIDETONE_EN);
}

/* Called with apm->lock held (or before the component is registered). */
static void apm_program_rate(struct apm_priv *apm, unsigned int ch)
{
	const struct apm_rate *r = apm->rate[ch];
	unsigned int hi = r->rate > 43000;

	regmap_write(apm->regmap, APM_RATE_PHASE(ch), r->phase);

	regmap_write(apm->regmap, APM_NCO_DATA, r->nco);
	regmap_set_bits(apm->regmap, APM_NCO_CTRL, APM_NCO_LOAD_RATE(ch));
	apm_write_adc_gain(apm, ch);
	regmap_clear_bits(apm->regmap, APM_NCO_CTRL, APM_NCO_LOAD_RATE(ch));

	regmap_write(apm->regmap, APM_NCO_DATA, APM_NCO_BASE_64K);
	regmap_set_bits(apm->regmap, APM_NCO_CTRL, APM_NCO_LOAD_BASE(ch));
	regmap_update_bits(apm->regmap, APM_DAC_CFG(ch), APM_DAC_CFG_HIRATE,
			   FIELD_PREP(APM_DAC_CFG_HIRATE, hi));
	regmap_update_bits(apm->regmap, APM_ADC_GAIN(ch), APM_ADC_GAIN_HIRATE,
			   FIELD_PREP(APM_ADC_GAIN_HIRATE, hi));
	regmap_clear_bits(apm->regmap, APM_NCO_CTRL, APM_NCO_LOAD_BASE(ch));
}

/*
 * The vendor driver only ever starts and stops both channels together, so
 * that is the only sequence known to work.  Do the same on the first and
 * last active stream instead of guessing at per-channel, per-direction
 * enables.  A channel with no DMA attached simply over/underruns its FIFO.
 * UNVERIFIED: that a FIFO which has over/underrun picks up cleanly when its
 * stream is started later.
 *
 * Measured in the vendor's running state: the hold bits 31:29 of 0x020 are
 * clear while bit 31 of 0x050 is still set, so the latter does not gate
 * channel 1, and this sequence (which leaves it set) is the vendor's.
 *
 * Like the vendor driver, set the rates of both channels each time, while
 * the channels are still held: whether the rate setting survives the hold
 * is not known.
 */
static void apm_start_all(struct apm_priv *apm)
{
	unsigned int ch;

	for (ch = 0; ch < APM_NUM_CH; ch++)
		apm_program_rate(apm, ch);

	regmap_write(apm->regmap, APM_IRQ_STATUS,
		     APM_IRQ_DAC_UFLOW(0) | APM_IRQ_DAC_UFLOW(1) |
		     APM_IRQ_ADC_OFLOW(0) | APM_IRQ_ADC_OFLOW(1));
	regmap_write(apm->regmap, APM_FIFO_FLUSH_RX(0), ~0);
	regmap_write(apm->regmap, APM_FIFO_FLUSH_RX(1), ~0);
	regmap_write(apm->regmap, APM_FIFO_FLUSH_TX, ~0);
	regmap_write(apm->regmap, APM_ENABLE, 0xf);
	regmap_set_bits(apm->regmap, APM_DMA_CTRL, APM_DMA_EN_CH01);
	udelay(25);
	regmap_clear_bits(apm->regmap, APM_FILT_CTRL(0),
			  APM_FILT_CH_HOLD(0) | APM_FILT_CH_HOLD(1));
}

static void apm_stop_all(struct apm_priv *apm)
{
	regmap_clear_bits(apm->regmap, APM_DMA_CTRL, APM_DMA_EN_CH01);
	regmap_set_bits(apm->regmap, APM_FILT_CTRL(1), BIT(31));
	regmap_set_bits(apm->regmap, APM_FILT_CTRL(0),
			APM_FILT_CH_HOLD(0) | APM_FILT_CH_HOLD(1));
}

/* Power-on defaults, in the order the vendor hardware layer writes them. */
static const struct reg_sequence apm_init_seq[] = {
	{ APM_DMA_CTRL, 0x00810204 },	/* burst of 4 words on all FIFOs */
	{ 0x20c, 0x00000004 },
	{ APM_FILT_CTRL(0), 0xe0000000, 50 },
	{ APM_LOOPBACK(0), 0x00000420 },
	{ APM_FILT_CTRL(0), 0xe0880000 },
	{ APM_SIDETONE(0), 0x00000000 },
	{ APM_DAC_CFG(0), 0x02130000 },
	{ APM_DAC_GAIN(0), 0x00337fff },
	{ 0x03c, 0x0000031d },
	{ APM_ADC_GAIN(0), 0x031314f8 },
	{ APM_RATE_PHASE(0), 0x20000000 },
	{ 0x048, 0x00000010 },
	{ 0x04c, 0x00005000 },
	{ APM_FILT_CTRL(1), 0x80000000, 50 },
	{ APM_LOOPBACK(1), 0x00000420 },
	{ APM_FILT_CTRL(1), 0x80880000 },
	{ APM_SIDETONE(1), 0x00000000 },
	{ APM_DAC_CFG(1), 0x02130000 },
	{ APM_DAC_GAIN(1), 0x00337fff },
	{ 0x06c, 0x0000031d },
	{ APM_ADC_GAIN(1), 0x031314f8 },
	{ APM_RATE_PHASE(1), 0x20000000 },
	{ 0x078, 0x00000010 },
	{ 0x07c, 0x00005000 },
	/* Third (capture-only) channel: parked exactly as the vendor does. */
	{ 0x21c, 0x80000000, 50 },
	{ 0x218, 0x00000400 },
	{ 0x21c, 0x80080000 },
	{ 0x284, 0x00000000 },
	{ 0x220, 0x00000205 },
	{ 0x224, 0x031314f8 },
	{ 0x228, 0x20000000 },
	{ 0x22c, 0x00000010 },
	/* Rate generators. */
	{ 0x0a0, 0x80000000 },
	{ APM_NCO_DATA, APM_NCO_BASE_64K },
	{ APM_NCO_CTRL, 0x00000006 },
	{ 0x230, 0x00000002 },
	{ 0x234, 0x00000000 },
	{ 0x088, 0x00000000 },
	{ 0x168, 0x00000000 },
	{ 0x164, 0x00000000 },
	{ 0x160, 0x80000000 },
};

static void apm_hw_init(struct apm_priv *apm)
{
	static const u32 unity_eq[APM_EQ_TAPS] = { 0x7fff };
	unsigned int ch, sub;

	/* Ack and mask everything. */
	udelay(50);
	regmap_write(apm->regmap, APM_IRQ_STATUS, 0xff);
	regmap_write(apm->regmap, APM_IRQ_MASK, 0);

	regmap_multi_reg_write(apm->regmap, apm_init_seq, ARRAY_SIZE(apm_init_seq));
	regmap_clear_bits(apm->regmap, 0x0a0, BIT(31));

	/*
	 * Analog side: ADCs off, all four output drivers in their muted mode,
	 * unknown bits cleared as the vendor does.
	 */
	regmap_clear_bits(apm->regmap, APM_DAC_PWR, GENMASK(8, 7));
	regmap_clear_bits(apm->regmap, APM_ANA_PWR, BIT(0) | APM_ANA_PWR_ADC_ALL);
	regmap_set_bits(apm->regmap, APM_OUT_CTRL, GENMASK(19, 12));
	/*
	 * The vendor driver ends its probe with every gain "muted", which
	 * leaves the output drivers powered down; the DACs and the microphone
	 * bias are switched by its separate analog power call.  Start from
	 * everything off, so that DAPM finds a known state.
	 *
	 * This deviates from the vendor on purpose.  Measured, the stock
	 * firmware keeps the whole analog section powered while the phone is
	 * idle (0x120 = 0x48: both ADCs and both microphone biases on; no
	 * power-down bit set in 0x134 or 0x13c) and silences an unused
	 * output driver with its mode field alone.
	 */
	regmap_set_bits(apm->regmap, APM_OUT_PWR, APM_OUT_PWR_DRV_ALL_N);
	regmap_set_bits(apm->regmap, APM_DAC_PWR,
			BIT(APM_DAC_PWR_N(0)) | BIT(APM_DAC_PWR_N(1)));
	regmap_set_bits(apm->regmap, APM_ANA_PWR, APM_ANA_PWR_MICBIAS_ALL_N);

	/*
	 * Microphone-detect sub-blocks at 0x400 and 0x500 (purpose of each
	 * field unknown; values copied).
	 */
	for (sub = 0x400; sub <= 0x500; sub += 0x100) {
		regmap_set_bits(apm->regmap, sub + 0x34, BIT(0));
		regmap_write(apm->regmap, sub + 0x38, 4);
		regmap_set_bits(apm->regmap, sub + 0x2c, BIT(0));
		regmap_update_bits(apm->regmap, sub + 0x28, GENMASK(2, 0), 4);
		regmap_update_bits(apm->regmap, sub + 0x20, GENMASK(2, 0), 4);
	}
	regmap_update_bits(apm->regmap, APM_NCO_CTRL, APM_NCO_LIMIT,
			   FIELD_PREP(APM_NCO_LIMIT, 7));

	for (ch = 0; ch < APM_NUM_CH; ch++) {
		/* Flat equalizers on both paths, then enable them. */
		apm_coef_write(apm, APM_COEF_ADC_EQ(ch), unity_eq, APM_EQ_TAPS);
		regmap_update_bits(apm->regmap, APM_FILT_CTRL(ch), APM_FILT_ADC_EQ_LEN,
				   FIELD_PREP(APM_FILT_ADC_EQ_LEN, APM_EQ_TAPS - 1));
		apm_coef_write(apm, APM_COEF_DAC_EQ(ch), unity_eq, APM_EQ_TAPS);
		regmap_update_bits(apm->regmap, APM_FILT_CTRL(ch), APM_FILT_DAC_EQ_LEN,
				   FIELD_PREP(APM_FILT_DAC_EQ_LEN, APM_EQ_TAPS - 1));
		regmap_set_bits(apm->regmap, APM_FILT_CTRL(ch),
				APM_FILT_ADC_EQ_EN | APM_FILT_DAC_EQ_EN);

		apm->gain[ch][APM_GAIN_ADC] = -APM_GAIN_DB_MIN;
		apm->gain[ch][APM_GAIN_DAC] = -APM_GAIN_DB_MIN;
		apm->gain[ch][APM_GAIN_SIDETONE] = 0;
		apm->rate[ch] = &apm_rates[0];
		apm_program_rate(apm, ch);
		apm_write_dac_gain(apm, ch);
		apm_write_sidetone_gain(apm, ch);
	}
}

/* ---- Controls ---- */

#define APM_CTL_PRIV(ch, kind)	((ch) | ((kind) << 8))
#define APM_CTL_CH(pv)		((pv) & 0xff)
#define APM_CTL_KIND(pv)	(((pv) >> 8) & 0xff)

static int apm_gain_info(struct snd_kcontrol *kcontrol,
			 struct snd_ctl_elem_info *uinfo)
{
	uinfo->type = SNDRV_CTL_ELEM_TYPE_INTEGER;
	uinfo->count = 1;
	uinfo->value.integer.min = 0;
	uinfo->value.integer.max = APM_GAIN_STEPS;
	return 0;
}

static int apm_gain_get(struct snd_kcontrol *kcontrol,
			struct snd_ctl_elem_value *ucontrol)
{
	struct snd_soc_component *component = snd_kcontrol_chip(kcontrol);
	struct apm_priv *apm = snd_soc_component_get_drvdata(component);
	unsigned long pv = kcontrol->private_value;

	ucontrol->value.integer.value[0] = apm->gain[APM_CTL_CH(pv)][APM_CTL_KIND(pv)];
	return 0;
}

static int apm_gain_put(struct snd_kcontrol *kcontrol,
			struct snd_ctl_elem_value *ucontrol)
{
	struct snd_soc_component *component = snd_kcontrol_chip(kcontrol);
	struct apm_priv *apm = snd_soc_component_get_drvdata(component);
	unsigned long pv = kcontrol->private_value;
	unsigned int ch = APM_CTL_CH(pv), kind = APM_CTL_KIND(pv);
	long val = ucontrol->value.integer.value[0];
	unsigned long flags;

	if (val < 0 || val > APM_GAIN_STEPS)
		return -EINVAL;
	if (apm->gain[ch][kind] == val)
		return 0;

	if (kind == APM_GAIN_SIDETONE) {
		/* Control writes are serialized by the control core. */
		apm->gain[ch][kind] = val;
		apm_write_sidetone_gain(apm, ch);
		return 1;
	}

	/* The capture gain depends on the rate and is rewritten with it. */
	spin_lock_irqsave(&apm->lock, flags);
	apm->gain[ch][kind] = val;
	if (kind == APM_GAIN_ADC)
		apm_write_adc_gain(apm, ch);
	else
		apm_write_dac_gain(apm, ch);
	spin_unlock_irqrestore(&apm->lock, flags);
	return 1;
}

static int apm_pga_info(struct snd_kcontrol *kcontrol,
			struct snd_ctl_elem_info *uinfo)
{
	uinfo->type = SNDRV_CTL_ELEM_TYPE_INTEGER;
	uinfo->count = 1;
	uinfo->value.integer.min = 0;
	uinfo->value.integer.max = 42;
	return 0;
}

static int apm_pga_get(struct snd_kcontrol *kcontrol,
		       struct snd_ctl_elem_value *ucontrol)
{
	struct snd_soc_component *component = snd_kcontrol_chip(kcontrol);
	struct apm_priv *apm = snd_soc_component_get_drvdata(component);
	unsigned int val;

	regmap_read(apm->regmap, APM_MIC_CTRL(kcontrol->private_value), &val);
	ucontrol->value.integer.value[0] = min_t(unsigned int,
						 FIELD_GET(APM_MIC_PGA, val), 42);
	return 0;
}

static int apm_pga_put(struct snd_kcontrol *kcontrol,
		       struct snd_ctl_elem_value *ucontrol)
{
	struct snd_soc_component *component = snd_kcontrol_chip(kcontrol);
	struct apm_priv *apm = snd_soc_component_get_drvdata(component);
	long val = ucontrol->value.integer.value[0];
	bool changed;
	int ret;

	if (val < 0 || val > 42)
		return -EINVAL;
	/* Below 21 dB the amplifier only has 3 dB steps. */
	if (val < 21)
		val -= val % 3;

	ret = regmap_update_bits_check(apm->regmap,
				       APM_MIC_CTRL(kcontrol->private_value),
				       APM_MIC_PGA, FIELD_PREP(APM_MIC_PGA, val),
				       &changed);
	return ret ? ret : changed;
}

static const DECLARE_TLV_DB_SCALE(apm_dig_tlv, APM_GAIN_DB_MIN * 100, 100, 0);
static const DECLARE_TLV_DB_SCALE(apm_pga_tlv, 0, 100, 0);
static const DECLARE_TLV_DB_SCALE(apm_out_tlv, -2100, 300, 0);

#define APM_GAIN_CTL(xname, ch, kind) {					\
	.iface = SNDRV_CTL_ELEM_IFACE_MIXER, .name = xname,		\
	.access = SNDRV_CTL_ELEM_ACCESS_TLV_READ |			\
		  SNDRV_CTL_ELEM_ACCESS_READWRITE,			\
	.tlv.p = apm_dig_tlv, .info = apm_gain_info,			\
	.get = apm_gain_get, .put = apm_gain_put,			\
	.private_value = APM_CTL_PRIV(ch, kind) }

#define APM_PGA_CTL(xname, ch) {					\
	.iface = SNDRV_CTL_ELEM_IFACE_MIXER, .name = xname,		\
	.access = SNDRV_CTL_ELEM_ACCESS_TLV_READ |			\
		  SNDRV_CTL_ELEM_ACCESS_READWRITE,			\
	.tlv.p = apm_pga_tlv, .info = apm_pga_info,			\
	.get = apm_pga_get, .put = apm_pga_put,				\
	.private_value = (ch) }

static const struct snd_kcontrol_new apm_controls[] = {
	APM_PGA_CTL("CH0 Mic Capture Volume", 0),
	APM_PGA_CTL("CH1 Mic Capture Volume", 1),
	APM_GAIN_CTL("CH0 Digital Capture Volume", 0, APM_GAIN_ADC),
	APM_GAIN_CTL("CH1 Digital Capture Volume", 1, APM_GAIN_ADC),
	APM_GAIN_CTL("CH0 Digital Playback Volume", 0, APM_GAIN_DAC),
	APM_GAIN_CTL("CH1 Digital Playback Volume", 1, APM_GAIN_DAC),
	APM_GAIN_CTL("CH0 Sidetone Volume", 0, APM_GAIN_SIDETONE),
	APM_GAIN_CTL("CH1 Sidetone Volume", 1, APM_GAIN_SIDETONE),
	SOC_SINGLE_TLV("CH0 Out A Playback Volume", APM_OUT_CTRL,
		       APM_OUT_ATTEN_SHIFT(0, 0), 7, 1, apm_out_tlv),
	SOC_SINGLE_TLV("CH0 Out B Playback Volume", APM_OUT_CTRL,
		       APM_OUT_ATTEN_SHIFT(0, 1), 7, 1, apm_out_tlv),
	SOC_SINGLE_TLV("CH1 Out A Playback Volume", APM_OUT_CTRL,
		       APM_OUT_ATTEN_SHIFT(1, 0), 7, 1, apm_out_tlv),
	SOC_SINGLE_TLV("CH1 Out B Playback Volume", APM_OUT_CTRL,
		       APM_OUT_ATTEN_SHIFT(1, 1), 7, 1, apm_out_tlv),
};

/* ---- DAPM ---- */

static const struct snd_kcontrol_new apm_ch0_sidetone_sw =
	SOC_DAPM_SINGLE("Switch", APM_SIDETONE(0), 15, 1, 0);
static const struct snd_kcontrol_new apm_ch1_sidetone_sw =
	SOC_DAPM_SINGLE("Switch", APM_SIDETONE(1), 15, 1, 0);

/* Start the hardware output ramp of one driver, as the vendor code does. */
static void apm_out_ramp(struct apm_priv *apm, unsigned int ch,
			 unsigned int out, bool up)
{
	unsigned int shift = APM_OUT_RAMP_SHIFT(ch, out) + 1;

	regmap_set_bits(apm->regmap, APM_DAC_PWR, APM_DAC_RAMP_EN(ch));
	regmap_update_bits(apm->regmap, APM_OUT_PWR, APM_OUT_RAMP_TIME(ch, out),
			   (APM_OUT_RAMP_MS / 20 - 1) << shift);
	regmap_update_bits(apm->regmap, APM_OUT_PWR, APM_OUT_RAMP_UP(ch, out),
			   up ? APM_OUT_RAMP_UP(ch, out) : 0);
}

/*
 * Besides its power bit each output driver has a 2 bit mode field; the
 * vendor writes 0 to run and 3 to mute, always just before the power bit.
 * The widget shift identifies the driver: see APM_OUT_PWR_DRV_N().
 *
 * The vendor also runs a 40 ms hardware ramp: "up" right after the DAC and
 * the drivers are powered, and "down" followed by a 100 ms wait before they
 * are powered off.  With the ramp enabled and "up" is therefore the only
 * state in which the outputs are known to pass audio, so reproduce it.
 * Measured in the vendor's running state: 0x13c = 0x60000000 (ramp enabled on
 * both channels) and 0x134 = 0x00003333 (all four drivers: direction up, 40
 * ms), which is what apm_out_ramp(up) writes.
 * UNVERIFIED: what the ramp acts on, and its reset state.
 */
static int apm_out_drv_event(struct snd_soc_dapm_widget *w,
			     struct snd_kcontrol *kcontrol, int event)
{
	struct snd_soc_component *component = snd_soc_dapm_to_component(w->dapm);
	struct apm_priv *apm = snd_soc_component_get_drvdata(component);
	unsigned int ch = (19 - w->shift) / 2;
	unsigned int out = (w->shift - 16) % 2;
	unsigned int shift = APM_OUT_MODE_SHIFT(ch, out);

	switch (event) {
	case SND_SOC_DAPM_PRE_PMU:
		regmap_update_bits(apm->regmap, APM_OUT_CTRL, 3 << shift,
				   APM_OUT_MODE_ON << shift);
		break;
	case SND_SOC_DAPM_POST_PMU:
		apm_out_ramp(apm, ch, out, true);
		break;
	case SND_SOC_DAPM_PRE_PMD:
		apm_out_ramp(apm, ch, out, false);
		msleep(APM_OUT_RAMP_WAIT_MS);
		regmap_update_bits(apm->regmap, APM_OUT_CTRL, 3 << shift,
				   APM_OUT_MODE_MUTE << shift);
		break;
	}
	return 0;
}

#define APM_OUT_DRV(name, ch, out)					\
	SND_SOC_DAPM_OUT_DRV_E(name, APM_OUT_PWR, APM_OUT_PWR_DRV_N(ch, out), \
			       1, NULL, 0, apm_out_drv_event,		\
			       SND_SOC_DAPM_PRE_PMU | SND_SOC_DAPM_POST_PMU | \
			       SND_SOC_DAPM_PRE_PMD)

static const struct snd_soc_dapm_widget apm_dapm_widgets[] = {
	SND_SOC_DAPM_INPUT("CH0 MIC A"),
	SND_SOC_DAPM_INPUT("CH0 MIC B"),
	SND_SOC_DAPM_INPUT("CH1 MIC A"),
	SND_SOC_DAPM_INPUT("CH1 MIC B"),

	SND_SOC_DAPM_SUPPLY("CH0 MICBIAS", APM_ANA_PWR, APM_ANA_PWR_MICBIAS0_N,
			    1, NULL, 0),
	SND_SOC_DAPM_SUPPLY("CH1 MICBIAS", APM_ANA_PWR, APM_ANA_PWR_MICBIAS1_N,
			    1, NULL, 0),

	/*
	 * Microphone input selector: input A when the bit is clear, input B
	 * when it is set.  Modelled as a stage in the input B path, so that
	 * the selector follows whichever input the card routes a live
	 * microphone to (B wins if both are).
	 */
	SND_SOC_DAPM_PGA("CH0 Mic B Select", APM_MIC_CTRL(0), APM_MIC_SEL_B, 0,
			 NULL, 0),
	SND_SOC_DAPM_PGA("CH1 Mic B Select", APM_MIC_CTRL(1), APM_MIC_SEL_B, 0,
			 NULL, 0),

	SND_SOC_DAPM_ADC("CH0 ADC", "CH0 Capture", APM_ANA_PWR, APM_ANA_PWR_ADC0, 0),
	SND_SOC_DAPM_ADC("CH1 ADC", "CH1 Capture", APM_ANA_PWR, APM_ANA_PWR_ADC1, 0),

	SND_SOC_DAPM_DAC("CH0 DAC", "CH0 Playback", APM_DAC_PWR, APM_DAC_PWR_N(0), 1),
	SND_SOC_DAPM_DAC("CH1 DAC", "CH1 Playback", APM_DAC_PWR, APM_DAC_PWR_N(1), 1),

	SND_SOC_DAPM_SWITCH("CH0 Sidetone", SND_SOC_NOPM, 0, 0, &apm_ch0_sidetone_sw),
	SND_SOC_DAPM_SWITCH("CH1 Sidetone", SND_SOC_NOPM, 0, 0, &apm_ch1_sidetone_sw),

	APM_OUT_DRV("CH0 Out A", 0, 0),
	APM_OUT_DRV("CH0 Out B", 0, 1),
	APM_OUT_DRV("CH1 Out A", 1, 0),
	APM_OUT_DRV("CH1 Out B", 1, 1),

	SND_SOC_DAPM_OUTPUT("CH0 OUT A"),
	SND_SOC_DAPM_OUTPUT("CH0 OUT B"),
	SND_SOC_DAPM_OUTPUT("CH1 OUT A"),
	SND_SOC_DAPM_OUTPUT("CH1 OUT B"),
};

static const struct snd_soc_dapm_route apm_dapm_routes[] = {
	{ "CH0 Mic B Select", NULL, "CH0 MIC B" },
	{ "CH1 Mic B Select", NULL, "CH1 MIC B" },
	{ "CH0 ADC", NULL, "CH0 MIC A" },
	{ "CH0 ADC", NULL, "CH0 Mic B Select" },
	{ "CH1 ADC", NULL, "CH1 MIC A" },
	{ "CH1 ADC", NULL, "CH1 Mic B Select" },

	{ "CH0 Sidetone", "Switch", "CH0 ADC" },
	{ "CH1 Sidetone", "Switch", "CH1 ADC" },
	{ "CH0 DAC", NULL, "CH0 Sidetone" },
	{ "CH1 DAC", NULL, "CH1 Sidetone" },

	{ "CH0 Out A", NULL, "CH0 DAC" },
	{ "CH0 Out B", NULL, "CH0 DAC" },
	{ "CH1 Out A", NULL, "CH1 DAC" },
	{ "CH1 Out B", NULL, "CH1 DAC" },
	{ "CH0 OUT A", NULL, "CH0 Out A" },
	{ "CH0 OUT B", NULL, "CH0 Out B" },
	{ "CH1 OUT A", NULL, "CH1 Out A" },
	{ "CH1 OUT B", NULL, "CH1 Out B" },
};

/* ---- PCM (dmaengine) ---- */

static const struct snd_pcm_hardware apm_pcm_hardware = {
	.info = SNDRV_PCM_INFO_MMAP | SNDRV_PCM_INFO_MMAP_VALID |
		SNDRV_PCM_INFO_INTERLEAVED | SNDRV_PCM_INFO_BLOCK_TRANSFER,
	.formats = SNDRV_PCM_FMTBIT_S16_LE,
	.channels_min = 1,
	.channels_max = 1,
	.period_bytes_min = 64,
	/*
	 * One period must fit one block of the DMA controller, so that it is
	 * never split in two blocks.  A block is 4095 units of the source
	 * width: 16380 bytes for capture (32-bit FIFO words) and more for
	 * playback.
	 */
	.period_bytes_max = 8 * 1024,
	.periods_min = 2,
	.periods_max = 64,
	.buffer_bytes_max = 64 * 1024,
};

static struct snd_soc_dai *apm_substream_dai(struct snd_pcm_substream *substream)
{
	struct snd_soc_pcm_runtime *rtd = snd_soc_substream_to_rtd(substream);

	return snd_soc_rtd_to_cpu(rtd, 0);
}

static int apm_pcm_open(struct snd_soc_component *component,
			struct snd_pcm_substream *substream)
{
	struct apm_priv *apm = snd_soc_component_get_drvdata(component);
	struct snd_soc_dai *dai = apm_substream_dai(substream);
	struct dma_chan *chan = apm->chan[dai->id][substream->stream];
	unsigned long stream = APM_STREAM(dai->id, substream->stream);
	int ret;

	if (!chan)
		return -ENODEV;

	/*
	 * A card may put one channel in several DAI links (headset and
	 * speakerphone share channel 1), but each FIFO has one DMA channel.
	 */
	if (test_and_set_bit(__ffs(stream), &apm->open))
		return -EBUSY;

	snd_soc_set_runtime_hwparams(substream, &apm_pcm_hardware);

	/*
	 * The FIFOs are accessed as 32 bit words holding two samples each, so
	 * every period must be a whole number of words.  Ask for whole 64 bit
	 * words (four samples): the DMA controller then accesses the buffer 64
	 * bits wide, which is how the vendor driver runs these FIFOs (measured
	 * control words 0x00904835 and 0x1a204827); with a period of an odd
	 * number of FIFO words it would fall back to 32 bit memory accesses.
	 */
	ret = snd_pcm_hw_constraint_step(substream->runtime, 0,
					 SNDRV_PCM_HW_PARAM_PERIOD_SIZE, 4);
	if (!ret)
		ret = snd_dmaengine_pcm_open(substream, chan);
	if (ret)
		clear_bit(__ffs(stream), &apm->open);

	return ret;
}

static int apm_pcm_close(struct snd_soc_component *component,
			 struct snd_pcm_substream *substream)
{
	struct apm_priv *apm = snd_soc_component_get_drvdata(component);
	struct snd_soc_dai *dai = apm_substream_dai(substream);

	snd_dmaengine_pcm_close(substream);
	clear_bit(__ffs(APM_STREAM(dai->id, substream->stream)), &apm->open);
	return 0;
}

static int apm_pcm_hw_params(struct snd_soc_component *component,
			     struct snd_pcm_substream *substream,
			     struct snd_pcm_hw_params *params)
{
	struct apm_priv *apm = snd_soc_component_get_drvdata(component);
	struct snd_soc_dai *dai = apm_substream_dai(substream);
	struct dma_slave_config config = { };
	int ret;

	ret = snd_hwparams_to_dma_slave_config(substream, params, &config);
	if (ret)
		return ret;

	snd_dmaengine_pcm_set_config_from_dai_data(substream,
			&apm->dma_data[dai->id][substream->stream], &config);

	return dmaengine_slave_config(apm->chan[dai->id][substream->stream],
				      &config);
}

static int apm_pcm_trigger(struct snd_soc_component *component,
			   struct snd_pcm_substream *substream, int cmd)
{
	return snd_dmaengine_pcm_trigger(substream, cmd);
}

static int apm_pcm_sync_stop(struct snd_soc_component *component,
			     struct snd_pcm_substream *substream)
{
	return snd_dmaengine_pcm_sync_stop(substream);
}

static snd_pcm_uframes_t apm_pcm_pointer(struct snd_soc_component *component,
					 struct snd_pcm_substream *substream)
{
	return snd_dmaengine_pcm_pointer(substream);
}

static int apm_pcm_new(struct snd_soc_component *component,
		       struct snd_soc_pcm_runtime *rtd)
{
	struct apm_priv *apm = snd_soc_component_get_drvdata(component);
	struct snd_soc_dai *dai = snd_soc_rtd_to_cpu(rtd, 0);
	struct snd_pcm_substream *substream;
	int stream;

	for_each_pcm_streams(stream) {
		struct dma_chan *chan = apm->chan[dai->id][stream];

		substream = rtd->pcm->streams[stream].substream;
		if (!substream || !chan)
			continue;
		snd_pcm_set_managed_buffer(substream, SNDRV_DMA_TYPE_DEV,
					   chan->device->dev,
					   apm_pcm_hardware.buffer_bytes_max,
					   apm_pcm_hardware.buffer_bytes_max);
	}
	return 0;
}

static int apm_component_probe(struct snd_soc_component *component)
{
	struct apm_priv *apm = snd_soc_component_get_drvdata(component);

	snd_soc_component_init_regmap(component, apm->regmap);
	return 0;
}

static const struct snd_soc_component_driver apm_component = {
	.name = "bcmring-apm",
	.probe = apm_component_probe,
	.controls = apm_controls,
	.num_controls = ARRAY_SIZE(apm_controls),
	.dapm_widgets = apm_dapm_widgets,
	.num_dapm_widgets = ARRAY_SIZE(apm_dapm_widgets),
	.dapm_routes = apm_dapm_routes,
	.num_dapm_routes = ARRAY_SIZE(apm_dapm_routes),
	.open = apm_pcm_open,
	.close = apm_pcm_close,
	.hw_params = apm_pcm_hw_params,
	.trigger = apm_pcm_trigger,
	.sync_stop = apm_pcm_sync_stop,
	.pointer = apm_pcm_pointer,
	.pcm_new = apm_pcm_new,
	.endianness = 1,
};

/* ---- DAIs ---- */

static int apm_dai_startup(struct snd_pcm_substream *substream,
			   struct snd_soc_dai *dai)
{
	/* One rate generator per channel serves both directions. */
	return snd_pcm_hw_constraint_list(substream->runtime, 0,
					  SNDRV_PCM_HW_PARAM_RATE,
					  &apm_rate_constraint);
}

static int apm_dai_hw_params(struct snd_pcm_substream *substream,
			     struct snd_pcm_hw_params *params,
			     struct snd_soc_dai *dai)
{
	struct apm_priv *apm = snd_soc_dai_get_drvdata(dai);
	unsigned int rate = params_rate(params);
	unsigned long flags;
	unsigned int i;

	for (i = 0; i < ARRAY_SIZE(apm_rates); i++)
		if (apm_rates[i].rate == rate)
			break;
	if (i == ARRAY_SIZE(apm_rates))
		return -EINVAL;

	/*
	 * The vendor driver refuses a rate change while the block runs and
	 * programs the rates just before it enables the channels;
	 * apm_start_all() does the latter.  While the block runs, leave an
	 * unchanged rate alone (the other direction of this channel may be
	 * streaming).  UNVERIFIED: changing the rate of an idle channel while
	 * the other channel streams, which the vendor never does.
	 */
	spin_lock_irqsave(&apm->lock, flags);
	if (apm->rate[dai->id] != &apm_rates[i] || !apm->active) {
		apm->rate[dai->id] = &apm_rates[i];
		apm_program_rate(apm, dai->id);
	}
	spin_unlock_irqrestore(&apm->lock, flags);

	return 0;
}

static int apm_dai_trigger(struct snd_pcm_substream *substream, int cmd,
			   struct snd_soc_dai *dai)
{
	struct apm_priv *apm = snd_soc_dai_get_drvdata(dai);
	unsigned long stream = APM_STREAM(dai->id, substream->stream);
	unsigned long flags;
	int ret = 0;

	/*
	 * The DMA ring is armed before this runs and torn down after it (the
	 * default ASoC trigger order), like the vendor's start; no priming is
	 * needed, see the notes.
	 */
	spin_lock_irqsave(&apm->lock, flags);
	switch (cmd) {
	case SNDRV_PCM_TRIGGER_START:
	case SNDRV_PCM_TRIGGER_RESUME:
	case SNDRV_PCM_TRIGGER_PAUSE_RELEASE:
		if (!apm->active)
			apm_start_all(apm);
		apm->active |= stream;
		break;
	case SNDRV_PCM_TRIGGER_STOP:
	case SNDRV_PCM_TRIGGER_SUSPEND:
	case SNDRV_PCM_TRIGGER_PAUSE_PUSH:
		if (apm->active == stream)
			apm_stop_all(apm);
		apm->active &= ~stream;
		break;
	default:
		ret = -EINVAL;
	}
	spin_unlock_irqrestore(&apm->lock, flags);

	return ret;
}

static const struct snd_soc_dai_ops apm_dai_ops = {
	.startup = apm_dai_startup,
	.hw_params = apm_dai_hw_params,
	.trigger = apm_dai_trigger,
};

#define APM_DAI(ch) {							\
	.name = "apm-ch" #ch,						\
	.id = ch,							\
	.playback = {							\
		.stream_name = "CH" #ch " Playback",			\
		.channels_min = 1, .channels_max = 1,			\
		.rates = SNDRV_PCM_RATE_KNOT,				\
		.formats = SNDRV_PCM_FMTBIT_S16_LE,			\
	},								\
	.capture = {							\
		.stream_name = "CH" #ch " Capture",			\
		.channels_min = 1, .channels_max = 1,			\
		.rates = SNDRV_PCM_RATE_KNOT,				\
		.formats = SNDRV_PCM_FMTBIT_S16_LE,			\
	},								\
	.ops = &apm_dai_ops,						\
	.symmetric_rate = 1,						\
}

static struct snd_soc_dai_driver apm_dais[APM_NUM_CH] = {
	APM_DAI(0),
	APM_DAI(1),
};

/* ---- Platform driver ---- */

static irqreturn_t apm_irq(int irq, void *data)
{
	struct apm_priv *apm = data;
	unsigned int status;

	regmap_read(apm->regmap, APM_IRQ_STATUS, &status);
	if (!status)
		return IRQ_NONE;
	regmap_write(apm->regmap, APM_IRQ_STATUS, status);

	/*
	 * Nothing is unmasked yet.  The microphone-detect bits are where
	 * headset jack/hook detection would be reported (snd_soc_jack); the
	 * over/underflow bits are FIFO error indications.
	 */
	dev_dbg_ratelimited(apm->dev, "irq status %#x\n", status);
	return IRQ_HANDLED;
}

static bool apm_volatile_reg(struct device *dev, unsigned int reg)
{
	switch (reg) {
	case APM_IRQ_STATUS:
	case 0x204:
	case 0x468:
	case 0x568:
		return true;
	default:
		return false;
	}
}

static const struct regmap_config apm_regmap_config = {
	.reg_bits = 32,
	.reg_stride = 4,
	.val_bits = 32,
	.max_register = APM_MAX_REGISTER,
	.volatile_reg = apm_volatile_reg,
	.cache_type = REGCACHE_NONE,
};

static const char * const apm_dma_names[APM_NUM_CH][2] = {
	[0] = { [SNDRV_PCM_STREAM_PLAYBACK] = "tx0", [SNDRV_PCM_STREAM_CAPTURE] = "rx0" },
	[1] = { [SNDRV_PCM_STREAM_PLAYBACK] = "tx1", [SNDRV_PCM_STREAM_CAPTURE] = "rx1" },
};

static void apm_clk_disable(void *data)
{
	clk_disable_unprepare(data);
}

static void apm_release_dma(void *data)
{
	struct apm_priv *apm = data;
	unsigned int ch, stream;

	for (ch = 0; ch < APM_NUM_CH; ch++)
		for (stream = 0; stream < 2; stream++)
			if (apm->chan[ch][stream])
				dma_release_channel(apm->chan[ch][stream]);
}

static int apm_request_dma(struct platform_device *pdev, struct apm_priv *apm)
{
	unsigned int ch, stream;
	int ret;

	ret = devm_add_action(&pdev->dev, apm_release_dma, apm);
	if (ret)
		return ret;

	for (ch = 0; ch < APM_NUM_CH; ch++) {
		for (stream = 0; stream < 2; stream++) {
			const char *name = apm_dma_names[ch][stream];
			struct snd_dmaengine_dai_dma_data *dd = &apm->dma_data[ch][stream];
			struct resource *res;
			struct dma_chan *chan;

			res = platform_get_resource_byname(pdev, IORESOURCE_MEM, name);
			if (!res)
				return dev_err_probe(&pdev->dev, -EINVAL,
						     "missing %s FIFO address\n", name);

			chan = dma_request_chan(&pdev->dev, name);
			if (IS_ERR(chan))
				return dev_err_probe(&pdev->dev, PTR_ERR(chan),
						     "no %s DMA channel\n", name);
			apm->chan[ch][stream] = chan;

			dd->addr = res->start;
			dd->addr_width = DMA_SLAVE_BUSWIDTH_4_BYTES;
			dd->maxburst = APM_FIFO_BURST;
		}
	}
	return 0;
}

static int apm_probe(struct platform_device *pdev)
{
	struct device *dev = &pdev->dev;
	struct apm_priv *apm;
	struct reset_control *rst;
	struct regmap *chipc;
	struct clk *parent;
	void __iomem *base;
	int irq, ret;

	apm = devm_kzalloc(dev, sizeof(*apm), GFP_KERNEL);
	if (!apm)
		return -ENOMEM;
	apm->dev = dev;
	spin_lock_init(&apm->lock);
	platform_set_drvdata(pdev, apm);

	base = devm_platform_ioremap_resource_byname(pdev, "core");
	if (IS_ERR(base))
		return PTR_ERR(base);

	apm->regmap = devm_regmap_init_mmio(dev, base, &apm_regmap_config);
	if (IS_ERR(apm->regmap))
		return PTR_ERR(apm->regmap);

	/*
	 * Get the DMA channels before any clock, reset, or register is
	 * touched: while the SoC's DMA controller has no working dmaengine
	 * driver this is where probing defers, and a deferred probe must not
	 * have reprogrammed the APM clocks.
	 */
	ret = apm_request_dma(pdev, apm);
	if (ret)
		return ret;

	/* The vendor code asks for 200 MHz on the parent and 100 MHz here. */
	parent = devm_clk_get_optional(dev, "apm200");
	if (IS_ERR(parent))
		return dev_err_probe(dev, PTR_ERR(parent), "apm200 clock\n");
	if (parent) {
		ret = clk_set_rate(parent, 200000000);
		if (ret)
			return dev_err_probe(dev, ret, "apm200 rate\n");
	}

	apm->clk = devm_clk_get(dev, "apm100");
	if (IS_ERR(apm->clk))
		return dev_err_probe(dev, PTR_ERR(apm->clk), "apm100 clock\n");
	ret = clk_set_rate(apm->clk, 100000000);
	if (ret)
		return dev_err_probe(dev, ret, "apm100 rate\n");
	ret = clk_prepare_enable(apm->clk);
	if (ret)
		return ret;
	ret = devm_add_action_or_reset(dev, apm_clk_disable, apm->clk);
	if (ret)
		return ret;

	/*
	 * The audio clock/channel enables (chip-control offset 0x7c) have no
	 * clock or reset abstraction; set them through the chip-control
	 * syscon regmap, which the clock driver owns and shares.  Then
	 * release the block resets (vendor: SoftReset2 &= ~0x2001), same
	 * order as the vendor driver.
	 */
	chipc = syscon_regmap_lookup_by_phandle_optional(dev->of_node, "brcm,chipc");
	if (IS_ERR(chipc))
		return dev_err_probe(dev, PTR_ERR(chipc), "chipc syscon\n");
	if (chipc)
		regmap_set_bits(chipc, CHIPC_AUDIO_ENABLE, CHIPC_AUDIO_EN_APM);

	rst = devm_reset_control_array_get_optional_exclusive(dev);
	if (IS_ERR(rst))
		return dev_err_probe(dev, PTR_ERR(rst), "resets\n");
	ret = reset_control_deassert(rst);
	if (ret)
		return dev_err_probe(dev, ret, "failed to release resets\n");

	apm_hw_init(apm);

	irq = platform_get_irq(pdev, 0);
	if (irq < 0)
		return irq;
	ret = devm_request_irq(dev, irq, apm_irq, 0, dev_name(dev), apm);
	if (ret)
		return ret;

	return devm_snd_soc_register_component(dev, &apm_component, apm_dais,
					       ARRAY_SIZE(apm_dais));
}

static const struct of_device_id apm_of_match[] = {
	{ .compatible = "brcm,bcm11107-apm" },
	{ }
};
MODULE_DEVICE_TABLE(of, apm_of_match);

static struct platform_driver apm_driver = {
	.probe = apm_probe,
	.driver = {
		.name = "bcmring-apm",
		.of_match_table = apm_of_match,
	},
};
module_platform_driver(apm_driver);

MODULE_DESCRIPTION("Broadcom BCMRING APM analog audio front end");
MODULE_LICENSE("GPL");
