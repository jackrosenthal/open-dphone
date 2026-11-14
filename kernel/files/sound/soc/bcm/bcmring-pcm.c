// SPDX-License-Identifier: GPL-2.0-only
/*
 * Broadcom BCMRING (BCM11107 family) PCM/TDM interface, as a mono ASoC DAI
 * for Bluetooth voice (SCO) on the Digium D65.
 *
 * NEVER RUN ON HARDWARE.  No D65 was available; everything here is taken
 * from the vendor 3.4.74 kernel of the D65 (PcmHw_* at 0xc001e450 and up, the
 * DMA device table entries "pcm0_rx" and "pcm0_tx") and its halaudio_pcm.ko.
 *
 * What the vendor code establishes:
 *
 *  - The block sits at 0x30600600 (a second instance at 0x30600700 is
 *    unused), in the APM's window.  It needs the APM 100 MHz clock, the NTP,
 *    APM, and PCM clock enables in chip-control word 0x7c, and one pulse of
 *    the PCM soft reset.
 *  - The control word's rate (bit 27), sample size (bit 23), bit-clock
 *    select (bits 22:20), loopback (bit 17), frame length (bits 10:0), and
 *    enable (bit 31) are read from the vendor functions that set them.  The
 *    names of the other bits (30, 29, 28, 26, 25, 24, and 18) are taken
 *    from the PCM block of the BCM63xx family, whose layout this block
 *    matches bit for bit where it can be compared; the vendor's values for
 *    them are used unchanged.  Under those names the SoC is the bit-clock
 *    and frame-sync master: 1.024 MHz, 8 or 16 kHz short frame sync, 16-bit
 *    samples MSB first, voice in time slot 0.  The Bluetooth controller
 *    therefore has to be the slave on both.
 *  - Samples move through two FIFOs with 32-bit words, by DMA request lines
 *    12 (receive) and 13 (transmit) of DMA controller 0, peripheral burst 8.
 *  - The FIFO stream always carries eight channels of 16 bits per frame,
 *    two frames at a time: eight words, word n holding channel n's samples
 *    of two consecutive frames in memory order (vendor extract_data and
 *    interleave_data).  Only channel 0 is enabled and mapped to a time
 *    slot.
 *  - The vendor runs both directions whenever the interface is prepared,
 *    with two 5 ms DMA blocks per direction.
 *
 * Because of the FIFO packing, the DMA buffers are private to the driver
 * and ALSA sees an ordinary mono S16_LE buffer: each DMA block interrupt
 * moves one block between the two.
 */

#include <linux/bitfield.h>
#include <linux/bits.h>
#include <linux/clk.h>
#include <linux/dma-mapping.h>
#include <linux/dmaengine.h>
#include <linux/io.h>
#include <linux/mfd/syscon.h>
#include <linux/module.h>
#include <linux/mutex.h>
#include <linux/of.h>
#include <linux/platform_device.h>
#include <linux/regmap.h>
#include <linux/reset.h>
#include <linux/spinlock.h>
#include <linux/string.h>

#include <sound/pcm.h>
#include <sound/pcm_params.h>
#include <sound/soc.h>

#define PCM_CTRL		0x00
#define  PCM_CTRL_ENABLE	BIT(31)
#define  PCM_CTRL_SLAVE		BIT(30)	/* name by analogy, see above */
#define  PCM_CTRL_CLK_INV	BIT(29)	/* name by analogy */
#define  PCM_CTRL_FS_INV	BIT(28)	/* name by analogy */
#define  PCM_CTRL_FS_16K	BIT(27)	/* clear = 8 kHz */
#define  PCM_CTRL_FS_LONG	BIT(26)	/* name by analogy */
#define  PCM_CTRL_FS_TRIG	BIT(25)	/* name by analogy */
#define  PCM_CTRL_DATA_OFF	BIT(24)	/* name by analogy */
#define  PCM_CTRL_DATA_16	BIT(23)	/* clear = 8-bit samples */
#define  PCM_CTRL_CLK_SEL	GENMASK(22, 20)	/* 8192 kHz >> n */
#define  PCM_CTRL_LSB_FIRST	BIT(18)	/* name by analogy */
#define  PCM_CTRL_LOOPBACK	BIT(17)
#define  PCM_CTRL_FRAME_BITS	GENMASK(10, 0)
#define PCM_CHAN_CTRL		0x04
#define  PCM_CHAN_RX(n)		BIT(n)
#define  PCM_CHAN_TX(n)		BIT(8 + (n))
#define PCM_INT_PENDING		0x08
#define PCM_INT_MASK		0x0c
/*
 * 0x10 to 0x20: clock generation.  Only the vendor's values are known; the
 * vendor's interface enable fails unless word 0x1c reads the same as word
 * 0x10.
 */
#define PCM_CLK_10		0x10
#define PCM_CLK_14		0x14
#define PCM_CLK_18		0x18
#define PCM_CLK_1C		0x1c
#define PCM_CLK_20		0x20
#define PCM_SLOT_MAP(slot)	(0x40 + 4 * ((slot) / 8))
#define  PCM_SLOT_SHIFT(slot)	(28 - 4 * ((slot) % 8))
#define  PCM_SLOT_VALID		8

/* Chip control (syscon) */
#define CHIPC_AUDIO_ENABLE	0x7c
#define  CHIPC_AUDIO_EN_NTP	BIT(4)
#define  CHIPC_AUDIO_EN_PCM0	BIT(5)
#define  CHIPC_AUDIO_EN_PCM1	BIT(6)
#define  CHIPC_AUDIO_EN_APM	BIT(7)

#define PCM_BITCLK_KHZ		1024
#define PCM_CLK_SEL_1024	3	/* 8192 kHz >> 3 */
#define PCM_FIFO_BURST		8
#define PCM_CHANNEL		0
#define PCM_TIME_SLOT		0

/* The FIFO stream: eight 32-bit words per two frames. */
#define PCM_WORDS_PER_PAIR	8
#define PCM_BYTES_PER_PAIR	(PCM_WORDS_PER_PAIR * sizeof(u32))
/* One DMA block is 5 ms, as in the vendor driver. */
#define PCM_BLOCK_MS		5
#define PCM_MAX_RATE		16000
#define PCM_BLOCK_BYTES(rate)	((rate) * PCM_BLOCK_MS / 1000 / 2 * PCM_BYTES_PER_PAIR)
#define PCM_BLOCKS		2
#define PCM_DMA_BYTES		(PCM_BLOCKS * PCM_BLOCK_BYTES(PCM_MAX_RATE))

#define TX	SNDRV_PCM_STREAM_PLAYBACK
#define RX	SNDRV_PCM_STREAM_CAPTURE

struct bcmring_pcm_dir {
	struct bcmring_pcm *pcm;
	struct dma_chan *chan;
	phys_addr_t fifo;
	dma_cookie_t cookie;
	void *buf;
	dma_addr_t buf_dma;

	/* Protected by bcmring_pcm.cb_lock. */
	struct snd_pcm_substream *substream;
	bool active;
	unsigned int pos;		/* frames, in the ALSA buffer */
	unsigned int period_pos;	/* frames since the last period */
};

struct bcmring_pcm {
	struct device *dev;
	void __iomem *base;
	struct clk *clk;

	/* Hardware state: sleeping context only. */
	struct mutex lock;
	bool running;
	unsigned int rate;
	unsigned int block_bytes;
	bool prepared[2];

	/*
	 * Held by the DMA callback for its whole run, including the period
	 * notification, so that taking and releasing it waits for a callback
	 * in flight.  Never taken inside the PCM stream lock.
	 */
	spinlock_t cb_lock;
	struct bcmring_pcm_dir dir[2];
};

/* ---- moving one DMA block ------------------------------------------------ */

/*
 * @block: block_bytes of FIFO words.  @frames mono S16 frames go to or come
 * from the ALSA buffer at dir->pos, which may wrap.
 */
static void bcmring_pcm_move(struct bcmring_pcm_dir *dir, void *block,
			     unsigned int frames, bool to_alsa)
{
	struct snd_pcm_runtime *runtime = dir->substream->runtime;
	s16 *ring = (s16 *)runtime->dma_area;
	unsigned int size = runtime->buffer_size;
	unsigned int pos = dir->pos;
	unsigned int i;
	s16 pair[2];

	for (i = 0; i < frames; i += 2) {
		void *word = block + (i / 2) * PCM_BYTES_PER_PAIR +
			     PCM_CHANNEL * sizeof(u32);

		if (to_alsa) {
			memcpy(pair, word, sizeof(pair));
			ring[pos] = pair[0];
			ring[(pos + 1) % size] = pair[1];
		} else {
			pair[0] = ring[pos];
			pair[1] = ring[(pos + 1) % size];
			memcpy(word, pair, sizeof(pair));
		}
		pos = (pos + 2) % size;
	}
	dir->pos = pos;
}

static void bcmring_pcm_dma_done(void *data)
{
	struct bcmring_pcm_dir *dir = data;
	struct bcmring_pcm *pcm = dir->pcm;
	bool capture = dir == &pcm->dir[RX];
	unsigned int block_bytes, frames, done;
	struct dma_tx_state state;
	unsigned long flags;
	void *block;

	spin_lock_irqsave(&pcm->cb_lock, flags);

	block_bytes = READ_ONCE(pcm->block_bytes);
	if (!block_bytes)
		goto out;

	/*
	 * The block that just finished is the one the transfer is not in
	 * now.  Reading the position, instead of counting callbacks, keeps
	 * this right when a callback is lost.
	 */
	dmaengine_tx_status(dir->chan, dir->cookie, &state);
	if (state.residue > PCM_BLOCKS * block_bytes)
		goto out;
	done = ((PCM_BLOCKS * block_bytes - state.residue) / block_bytes + 1) %
	       PCM_BLOCKS;
	block = dir->buf + done * block_bytes;
	frames = block_bytes / PCM_BYTES_PER_PAIR * 2;

	if (!READ_ONCE(dir->active)) {
		/* Nothing to play: the block goes out again as silence. */
		if (!capture)
			memset(block, 0, block_bytes);
		goto out;
	}

	bcmring_pcm_move(dir, block, frames, capture);

	dir->period_pos += frames;
	if (dir->period_pos >= dir->substream->runtime->period_size) {
		dir->period_pos %= dir->substream->runtime->period_size;
		snd_pcm_period_elapsed(dir->substream);
	}
out:
	spin_unlock_irqrestore(&pcm->cb_lock, flags);
}

/* ---- hardware -------------------------------------------------------------- */

static void bcmring_pcm_update(struct bcmring_pcm *pcm, unsigned int reg,
			       u32 clear, u32 set)
{
	writel((readl(pcm->base + reg) & ~clear) | set, pcm->base + reg);
}

/* The vendor's PcmHw_init(), value for value. */
static void bcmring_pcm_hw_init(struct bcmring_pcm *pcm)
{
	unsigned int slot;

	writel(0x80, pcm->base + PCM_CLK_18);
	for (slot = 0; slot < 128; slot += 8)
		writel(0, pcm->base + PCM_SLOT_MAP(slot));
	writel(0, pcm->base + PCM_CTRL);
	writel(0, pcm->base + PCM_CHAN_CTRL);
	writel(0, pcm->base + PCM_INT_MASK);
	writel(7, pcm->base + PCM_INT_PENDING);
	writel(0x29f16b12, pcm->base + PCM_CLK_10);
	writel(0x80000000, pcm->base + PCM_CLK_14);
	writel(0x31, pcm->base + PCM_CLK_18);
	writel(0x00080008, pcm->base + PCM_CLK_20);
	bcmring_pcm_update(pcm, PCM_CLK_18, 0, 0x40);
}

static int bcmring_pcm_start_dma(struct bcmring_pcm *pcm, int stream)
{
	struct bcmring_pcm_dir *dir = &pcm->dir[stream];
	struct dma_async_tx_descriptor *desc;

	memset(dir->buf, 0, PCM_DMA_BYTES);

	desc = dmaengine_prep_dma_cyclic(dir->chan, dir->buf_dma,
					 PCM_BLOCKS * pcm->block_bytes,
					 pcm->block_bytes,
					 stream == TX ? DMA_MEM_TO_DEV :
							DMA_DEV_TO_MEM,
					 DMA_PREP_INTERRUPT | DMA_CTRL_ACK);
	if (!desc)
		return -ENOMEM;
	desc->callback = bcmring_pcm_dma_done;
	desc->callback_param = dir;
	dir->cookie = dmaengine_submit(desc);
	if (dma_submit_error(dir->cookie))
		return -EIO;
	dma_async_issue_pending(dir->chan);
	return 0;
}

static void bcmring_pcm_hw_stop(struct bcmring_pcm *pcm)
{
	lockdep_assert_held(&pcm->lock);

	if (!pcm->running)
		return;

	/* Vendor order: transmit DMA, channels, interface, receive DMA, slot. */
	dmaengine_terminate_sync(pcm->dir[TX].chan);
	bcmring_pcm_update(pcm, PCM_CHAN_CTRL,
			   PCM_CHAN_RX(PCM_CHANNEL) | PCM_CHAN_TX(PCM_CHANNEL),
			   0);
	bcmring_pcm_update(pcm, PCM_CTRL, PCM_CTRL_ENABLE, 0);
	dmaengine_terminate_sync(pcm->dir[RX].chan);
	bcmring_pcm_update(pcm, PCM_SLOT_MAP(PCM_TIME_SLOT),
			   0xf << PCM_SLOT_SHIFT(PCM_TIME_SLOT), 0);

	WRITE_ONCE(pcm->block_bytes, 0);
	pcm->running = false;
}

static int bcmring_pcm_hw_start(struct bcmring_pcm *pcm, unsigned int rate)
{
	u32 ctrl;
	int ret;

	lockdep_assert_held(&pcm->lock);

	if (pcm->running)
		return rate == pcm->rate ? 0 : -EBUSY;

	pcm->rate = rate;
	WRITE_ONCE(pcm->block_bytes, PCM_BLOCK_BYTES(rate));

	/*
	 * The vendor's configuration for this board (PcmHw_ConfigSet,
	 * PcmHw_FsFreqSet, and PcmHw_DataSizeSet), with the frame length
	 * that its interface enable computes: bit clock over frame rate.
	 */
	ctrl = PCM_CTRL_CLK_INV | PCM_CTRL_FS_TRIG | PCM_CTRL_DATA_OFF |
	       PCM_CTRL_DATA_16 |
	       FIELD_PREP(PCM_CTRL_CLK_SEL, PCM_CLK_SEL_1024) |
	       FIELD_PREP(PCM_CTRL_FRAME_BITS, PCM_BITCLK_KHZ * 1000 / rate);
	if (rate == 16000)
		ctrl |= PCM_CTRL_FS_16K;
	writel(ctrl, pcm->base + PCM_CTRL);

	ret = bcmring_pcm_start_dma(pcm, TX);
	if (ret)
		goto err;
	ret = bcmring_pcm_start_dma(pcm, RX);
	if (ret)
		goto err_tx;

	bcmring_pcm_update(pcm, PCM_SLOT_MAP(PCM_TIME_SLOT),
			   0xf << PCM_SLOT_SHIFT(PCM_TIME_SLOT),
			   (PCM_SLOT_VALID | PCM_CHANNEL) <<
			   PCM_SLOT_SHIFT(PCM_TIME_SLOT));

	writel(ctrl | PCM_CTRL_ENABLE, pcm->base + PCM_CTRL);
	if (readl(pcm->base + PCM_CLK_1C) != readl(pcm->base + PCM_CLK_10)) {
		/* The vendor refuses to go on here; what it means is unknown. */
		dev_err(pcm->dev, "clock check failed: 0x1c reads %#x, 0x10 reads %#x\n",
			readl(pcm->base + PCM_CLK_1C),
			readl(pcm->base + PCM_CLK_10));
		ret = -EIO;
		goto err_rx;
	}
	bcmring_pcm_update(pcm, PCM_CHAN_CTRL, 0,
			   PCM_CHAN_RX(PCM_CHANNEL) | PCM_CHAN_TX(PCM_CHANNEL));

	pcm->running = true;
	return 0;

err_rx:
	writel(ctrl, pcm->base + PCM_CTRL);
	bcmring_pcm_update(pcm, PCM_SLOT_MAP(PCM_TIME_SLOT),
			   0xf << PCM_SLOT_SHIFT(PCM_TIME_SLOT), 0);
	dmaengine_terminate_sync(pcm->dir[RX].chan);
err_tx:
	dmaengine_terminate_sync(pcm->dir[TX].chan);
err:
	WRITE_ONCE(pcm->block_bytes, 0);
	return ret;
}

/* ---- PCM operations -------------------------------------------------------- */

static const struct snd_pcm_hardware bcmring_pcm_hardware = {
	.info = SNDRV_PCM_INFO_MMAP | SNDRV_PCM_INFO_MMAP_VALID |
		SNDRV_PCM_INFO_INTERLEAVED | SNDRV_PCM_INFO_BLOCK_TRANSFER |
		SNDRV_PCM_INFO_BATCH,
	.formats = SNDRV_PCM_FMTBIT_S16_LE,
	.rates = SNDRV_PCM_RATE_8000 | SNDRV_PCM_RATE_16000,
	.rate_min = 8000,
	.rate_max = 16000,
	.channels_min = 1,
	.channels_max = 1,
	.buffer_bytes_max = 32 * 1024,
	/* one 5 ms DMA block at 8 kHz */
	.period_bytes_min = 80,
	.period_bytes_max = 16 * 1024,
	.periods_min = 2,
	.periods_max = 64,
};

static int bcmring_pcm_open(struct snd_soc_component *component,
			    struct snd_pcm_substream *substream)
{
	struct bcmring_pcm *pcm = snd_soc_component_get_drvdata(component);
	struct bcmring_pcm_dir *dir = &pcm->dir[substream->stream];
	unsigned long flags;

	snd_soc_set_runtime_hwparams(substream, &bcmring_pcm_hardware);

	spin_lock_irqsave(&pcm->cb_lock, flags);
	dir->substream = substream;
	dir->active = false;
	spin_unlock_irqrestore(&pcm->cb_lock, flags);

	/* Periods are whole 5 ms DMA blocks at either rate. */
	return snd_pcm_hw_constraint_step(substream->runtime, 0,
					  SNDRV_PCM_HW_PARAM_PERIOD_TIME,
					  PCM_BLOCK_MS * 1000);
}

static int bcmring_pcm_close(struct snd_soc_component *component,
			     struct snd_pcm_substream *substream)
{
	struct bcmring_pcm *pcm = snd_soc_component_get_drvdata(component);
	struct bcmring_pcm_dir *dir = &pcm->dir[substream->stream];
	unsigned long flags;

	spin_lock_irqsave(&pcm->cb_lock, flags);
	dir->active = false;
	dir->substream = NULL;
	spin_unlock_irqrestore(&pcm->cb_lock, flags);
	return 0;
}

static int bcmring_pcm_prepare(struct snd_soc_component *component,
			       struct snd_pcm_substream *substream)
{
	struct bcmring_pcm *pcm = snd_soc_component_get_drvdata(component);
	struct bcmring_pcm_dir *dir = &pcm->dir[substream->stream];
	unsigned long flags;
	int ret;

	spin_lock_irqsave(&pcm->cb_lock, flags);
	dir->active = false;
	dir->pos = 0;
	dir->period_pos = 0;
	spin_unlock_irqrestore(&pcm->cb_lock, flags);

	/*
	 * Both directions run from the first prepare to the last hw_free,
	 * as in the vendor driver; a stream that is not triggered sends
	 * silence or drops what arrives.
	 */
	mutex_lock(&pcm->lock);
	ret = bcmring_pcm_hw_start(pcm, substream->runtime->rate);
	if (!ret)
		pcm->prepared[substream->stream] = true;
	mutex_unlock(&pcm->lock);
	return ret;
}

static int bcmring_pcm_hw_free(struct snd_soc_component *component,
			       struct snd_pcm_substream *substream)
{
	struct bcmring_pcm *pcm = snd_soc_component_get_drvdata(component);

	mutex_lock(&pcm->lock);
	pcm->prepared[substream->stream] = false;
	if (!pcm->prepared[TX] && !pcm->prepared[RX])
		bcmring_pcm_hw_stop(pcm);
	mutex_unlock(&pcm->lock);
	return 0;
}

static int bcmring_pcm_trigger(struct snd_soc_component *component,
			       struct snd_pcm_substream *substream, int cmd)
{
	struct bcmring_pcm *pcm = snd_soc_component_get_drvdata(component);
	struct bcmring_pcm_dir *dir = &pcm->dir[substream->stream];

	/*
	 * Called under the stream lock, so cb_lock is out of reach here; the
	 * flag is one word, read once per callback.
	 */
	switch (cmd) {
	case SNDRV_PCM_TRIGGER_START:
	case SNDRV_PCM_TRIGGER_RESUME:
	case SNDRV_PCM_TRIGGER_PAUSE_RELEASE:
		WRITE_ONCE(dir->active, true);
		return 0;
	case SNDRV_PCM_TRIGGER_STOP:
	case SNDRV_PCM_TRIGGER_SUSPEND:
	case SNDRV_PCM_TRIGGER_PAUSE_PUSH:
		WRITE_ONCE(dir->active, false);
		return 0;
	}
	return -EINVAL;
}

static int bcmring_pcm_sync_stop(struct snd_soc_component *component,
				 struct snd_pcm_substream *substream)
{
	struct bcmring_pcm *pcm = snd_soc_component_get_drvdata(component);
	unsigned long flags;

	/* Wait for a callback that saw the stream active. */
	spin_lock_irqsave(&pcm->cb_lock, flags);
	spin_unlock_irqrestore(&pcm->cb_lock, flags);
	return 0;
}

static snd_pcm_uframes_t bcmring_pcm_pointer(struct snd_soc_component *component,
					     struct snd_pcm_substream *substream)
{
	struct bcmring_pcm *pcm = snd_soc_component_get_drvdata(component);

	return READ_ONCE(pcm->dir[substream->stream].pos);
}

static int bcmring_pcm_new(struct snd_soc_component *component,
			   struct snd_soc_pcm_runtime *rtd)
{
	snd_pcm_set_managed_buffer_all(rtd->pcm, SNDRV_DMA_TYPE_VMALLOC, NULL,
				       0, 0);
	return 0;
}

static const struct snd_soc_component_driver bcmring_pcm_component = {
	.name = "bcmring-pcm",
	.open = bcmring_pcm_open,
	.close = bcmring_pcm_close,
	.prepare = bcmring_pcm_prepare,
	.hw_free = bcmring_pcm_hw_free,
	.trigger = bcmring_pcm_trigger,
	.sync_stop = bcmring_pcm_sync_stop,
	.pointer = bcmring_pcm_pointer,
	.pcm_construct = bcmring_pcm_new,
};

static struct snd_soc_dai_driver bcmring_pcm_dai = {
	.name = "bcmring-pcm",
	.playback = {
		.stream_name = "PCM Playback",
		.channels_min = 1,
		.channels_max = 1,
		.rates = SNDRV_PCM_RATE_8000 | SNDRV_PCM_RATE_16000,
		.formats = SNDRV_PCM_FMTBIT_S16_LE,
	},
	.capture = {
		.stream_name = "PCM Capture",
		.channels_min = 1,
		.channels_max = 1,
		.rates = SNDRV_PCM_RATE_8000 | SNDRV_PCM_RATE_16000,
		.formats = SNDRV_PCM_FMTBIT_S16_LE,
	},
	/* One frame sync serves both directions. */
	.symmetric_rate = 1,
};

/* ---- probe ----------------------------------------------------------------- */

static const char * const bcmring_pcm_dma_names[2] = {
	[TX] = "tx",
	[RX] = "rx",
};

static void bcmring_pcm_release(void *data)
{
	struct bcmring_pcm *pcm = data;
	int i;

	for (i = 0; i < 2; i++) {
		struct bcmring_pcm_dir *dir = &pcm->dir[i];

		if (dir->buf)
			dma_free_coherent(dir->chan->device->dev,
					  PCM_DMA_BYTES, dir->buf,
					  dir->buf_dma);
		if (dir->chan)
			dma_release_channel(dir->chan);
	}
}

static int bcmring_pcm_request_dma(struct platform_device *pdev,
				   struct bcmring_pcm *pcm)
{
	struct device *dev = &pdev->dev;
	int i, ret;

	ret = devm_add_action(dev, bcmring_pcm_release, pcm);
	if (ret)
		return ret;

	for (i = 0; i < 2; i++) {
		struct bcmring_pcm_dir *dir = &pcm->dir[i];
		struct dma_slave_config cfg = { };
		struct resource *res;
		struct dma_chan *chan;

		res = platform_get_resource_byname(pdev, IORESOURCE_MEM,
						   bcmring_pcm_dma_names[i]);
		if (!res)
			return dev_err_probe(dev, -EINVAL, "no %s FIFO\n",
					     bcmring_pcm_dma_names[i]);
		dir->pcm = pcm;
		dir->fifo = res->start;

		chan = dma_request_chan(dev, bcmring_pcm_dma_names[i]);
		if (IS_ERR(chan))
			return dev_err_probe(dev, PTR_ERR(chan),
					     "%s DMA channel\n",
					     bcmring_pcm_dma_names[i]);
		dir->chan = chan;

		/* Vendor "pcm0_rx"/"pcm0_tx": 32-bit FIFO words, burst 8. */
		if (i == TX) {
			cfg.direction = DMA_MEM_TO_DEV;
			cfg.dst_addr = dir->fifo;
			cfg.dst_addr_width = DMA_SLAVE_BUSWIDTH_4_BYTES;
			cfg.dst_maxburst = PCM_FIFO_BURST;
		} else {
			cfg.direction = DMA_DEV_TO_MEM;
			cfg.src_addr = dir->fifo;
			cfg.src_addr_width = DMA_SLAVE_BUSWIDTH_4_BYTES;
			cfg.src_maxburst = PCM_FIFO_BURST;
		}
		ret = dmaengine_slave_config(chan, &cfg);
		if (ret)
			return dev_err_probe(dev, ret, "%s DMA configuration\n",
					     bcmring_pcm_dma_names[i]);

		dir->buf = dma_alloc_coherent(chan->device->dev, PCM_DMA_BYTES,
					      &dir->buf_dma, GFP_KERNEL);
		if (!dir->buf)
			return -ENOMEM;
	}
	return 0;
}

static void bcmring_pcm_clk_disable(void *data)
{
	clk_disable_unprepare(data);
}

static int bcmring_pcm_probe(struct platform_device *pdev)
{
	struct device *dev = &pdev->dev;
	struct reset_control *rst;
	struct bcmring_pcm *pcm;
	struct regmap *chipc;
	struct clk *parent;
	int ret;

	pcm = devm_kzalloc(dev, sizeof(*pcm), GFP_KERNEL);
	if (!pcm)
		return -ENOMEM;
	pcm->dev = dev;
	mutex_init(&pcm->lock);
	spin_lock_init(&pcm->cb_lock);
	platform_set_drvdata(pdev, pcm);

	pcm->base = devm_platform_ioremap_resource_byname(pdev, "core");
	if (IS_ERR(pcm->base))
		return PTR_ERR(pcm->base);

	/*
	 * DMA channels first: this is where probing defers, and it must not
	 * have touched clocks or the block by then.
	 */
	ret = bcmring_pcm_request_dma(pdev, pcm);
	if (ret)
		return ret;

	/*
	 * Everything the block needs is switched on here, before its first
	 * register access, and nothing is assumed to be on already: after a
	 * cold boot none of it is.  The rates are the vendor's, the same
	 * that the APM driver asks for.
	 */
	parent = devm_clk_get_optional(dev, "apm200");
	if (IS_ERR(parent))
		return dev_err_probe(dev, PTR_ERR(parent), "apm200 clock\n");
	if (parent) {
		ret = clk_set_rate(parent, 200000000);
		if (ret)
			return dev_err_probe(dev, ret, "apm200 rate\n");
	}

	pcm->clk = devm_clk_get(dev, "apm100");
	if (IS_ERR(pcm->clk))
		return dev_err_probe(dev, PTR_ERR(pcm->clk), "apm100 clock\n");
	ret = clk_set_rate(pcm->clk, 100000000);
	if (ret)
		return dev_err_probe(dev, ret, "apm100 rate\n");
	ret = clk_prepare_enable(pcm->clk);
	if (ret)
		return ret;
	ret = devm_add_action_or_reset(dev, bcmring_pcm_clk_disable, pcm->clk);
	if (ret)
		return ret;

	/* Vendor pcmInit(): word 0x7c |= 0x90, then |= 0x60. */
	chipc = syscon_regmap_lookup_by_phandle(dev->of_node, "brcm,chipc");
	if (IS_ERR(chipc))
		return dev_err_probe(dev, PTR_ERR(chipc), "chipc syscon\n");
	regmap_set_bits(chipc, CHIPC_AUDIO_ENABLE,
			CHIPC_AUDIO_EN_NTP | CHIPC_AUDIO_EN_APM);
	regmap_set_bits(chipc, CHIPC_AUDIO_ENABLE,
			CHIPC_AUDIO_EN_PCM0 | CHIPC_AUDIO_EN_PCM1);

	/* Vendor: set the PCM soft reset, then release it. */
	rst = devm_reset_control_get_exclusive(dev, NULL);
	if (IS_ERR(rst))
		return dev_err_probe(dev, PTR_ERR(rst), "reset\n");
	ret = reset_control_assert(rst);
	if (!ret)
		ret = reset_control_deassert(rst);
	if (ret)
		return dev_err_probe(dev, ret, "failed to pulse the reset\n");

	bcmring_pcm_hw_init(pcm);

	return devm_snd_soc_register_component(dev, &bcmring_pcm_component,
					       &bcmring_pcm_dai, 1);
}

static const struct of_device_id bcmring_pcm_of_match[] = {
	{ .compatible = "brcm,bcm11107-pcm" },
	{ }
};
MODULE_DEVICE_TABLE(of, bcmring_pcm_of_match);

static struct platform_driver bcmring_pcm_driver = {
	.probe = bcmring_pcm_probe,
	.driver = {
		.name = "bcmring-pcm",
		.of_match_table = bcmring_pcm_of_match,
	},
};
module_platform_driver(bcmring_pcm_driver);

MODULE_DESCRIPTION("Broadcom BCMRING PCM/TDM interface (Bluetooth voice)");
MODULE_LICENSE("GPL");
