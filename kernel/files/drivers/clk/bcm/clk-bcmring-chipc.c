// SPDX-License-Identifier: GPL-2.0-only
/*
 * Broadcom BCMRING (BCM11107 family: BCM11109/11170/11188/11189, ...)
 * chip-control clock and reset controller.
 *
 * The chip-control block also holds pin multiplexing (0x114 and up, handled
 * by pinctrl-single child nodes) and a few miscellaneous registers that other
 * drivers need (audio enables at 0x7c, the dummy read the Ethernet switch
 * code performs).  To keep every read-modify-write of the block under one
 * lock, this driver creates the MMIO regmap, uses it for all of its own
 * accesses, and registers it as the syscon regmap of its node.  The node
 * must therefore NOT carry the generic "syscon" compatible, or a consumer
 * probing first could make the syscon core create a second, independently
 * locked regmap.
 *
 * Register layout, bit definitions, and the rate-change sequence are adapted
 * from the GPL mach-bcmring code that was in mainline Linux up to v3.5
 * (arch/arm/mach-bcmring/csp/chipc/chipcHw.c and
 * arch/arm/mach-bcmring/include/mach/csp/chipcHw_{reg,inline}.h,
 * Copyright 2003 - 2008 Broadcom Corporation), restructured for the common
 * clock framework.  The clock-ID-to-register table and the bus-interface
 * gate handling were checked against the vendor 3.4.74 kernel shipped on the
 * Digium D60.
 *
 * Model:
 *   xtal (25 MHz, from DT)
 *     pll1 = xtal * NDIV1        pll2 = xtal * NDIV2          (read-only)
 *       PLL channels: pll / MDIV, gate, bypass-to-xtal
 *         divider clocks: parent / DIV, gate, bypass-to-xtal
 *           bus-interface gates (children of the bus clock)
 *
 * The bypass bit is not modelled as a mux: a bypassed clock simply reports
 * the crystal rate.
 */

#include <linux/bitfield.h>
#include <linux/bits.h>
#include <linux/clk.h>
#include <linux/clk-provider.h>
#include <linux/delay.h>
#include <linux/io.h>
#include <linux/mfd/syscon.h>
#include <linux/module.h>
#include <linux/of.h>
#include <linux/of_platform.h>
#include <linux/platform_device.h>
#include <linux/regmap.h>
#include <linux/reset-controller.h>
#include <linux/slab.h>

#include <dt-bindings/clock/brcm,bcmring-chipc.h>

/* Register offsets (vendor chipcHw_REG_t) */
#define CHIPC_CHIPID		0x00
#define CHIPC_DDR_CLK		0x04
#define CHIPC_ARM_CLK		0x08
#define CHIPC_ESW_CLK		0x0c
#define CHIPC_VPM_CLK		0x10
#define CHIPC_ESW125_CLK	0x14
#define CHIPC_UART_CLK		0x18
#define CHIPC_SDIO0_CLK		0x1c
#define CHIPC_SDIO1_CLK		0x20
#define CHIPC_SPI_CLK		0x24
#define CHIPC_ETM_CLK		0x28
#define CHIPC_ACLK_CLK		0x2c
#define CHIPC_OTP_CLK		0x30
#define CHIPC_I2C_CLK		0x34
#define CHIPC_I2S0_CLK		0x38
#define CHIPC_RTBUS_CLK		0x3c
#define CHIPC_APM100_CLK	0x44
#define CHIPC_TSC_CLK		0x48
#define CHIPC_LED_CLK		0x4c
#define CHIPC_USB_CLK		0x50
#define CHIPC_LCD_CLK		0x54
#define CHIPC_APM_CLK		0x58
#define CHIPC_BUSINTF_CLK	0x5c
#define CHIPC_PLL_PREDIV	0x68
#define CHIPC_PLL_DIVIDER	0x6c
#define CHIPC_I2S1_CLK		0x78
#define CHIPC_SOFT_RESET1	0x80
#define CHIPC_SOFT_RESET2	0x84
#define CHIPC_PLL_PREDIV2	0xa8

#define CHIPID_REV_MASK		GENMASK(11, 0)
#define CHIPID_REV_A0		0xa00

/* PLL channel and divider clock registers */
#define CLK_POWER_DOWN		BIT(19)	/* PLL2 channels only */
#define CLK_SOURCE_GPIO		BIT(18)
#define CLK_BYPASS		BIT(17)	/* output = crystal */
#define CLK_OUTPUT_ENABLE	BIT(16)	/* not valid for DDR and VPM */
#define CLK_DIV_MASK		GENMASK(7, 0)	/* 0 means 256 */

#define PLL_PREDIV_NDIV_MASK	GENMASK(16, 8)
#define PLL_PREDIV_MODE_MASK	GENMASK(22, 20)	/* 0 = integer-N */
#define PLL_PREDIV_MODE_MASH_1_8	3	/* sigma-delta "1/8" modes */
#define PLL_PREDIV_MODE_MFB_1_8	4
#define PLL_DIVIDER_M1DIV_MASK	GENMASK(31, 24)	/* DDR channel divider */
#define PLL_DIVIDER_FRAC_MASK	GENMASK(23, 0)	/* PLL1 fractional NDIV */

enum bcmring_clk_kind {
	KIND_PLL,
	KIND_PLLCH,
	KIND_DIV,
	KIND_BUSIF,
};

/* per-clock quirks */
#define Q_RO		BIT(0)	/* no rate change */
#define Q_DDR		BIT(1)	/* divider in PLL_DIVIDER; read-only */
#define Q_PWRDN		BIT(2)	/* has a power-down bit (PLL2 channels) */
#define Q_LCD		BIT(3)	/* fixed /2 prescaler from chip rev B0 on */

#define PARENT_XTAL	0xff

struct bcmring_chipc;

struct bcmring_clk {
	struct clk_hw hw;
	struct bcmring_chipc *chipc;
	u16 reg;
	u8 kind;
	u8 bit;
	u8 quirks;
};

struct bcmring_chipc {
	struct regmap *regmap;	/* fast_io: usable from atomic clk callbacks */
	unsigned long xtal_rate;
	bool rev_a0;
	struct reset_controller_dev rcdev;
	struct bcmring_clk clks[BCMRING_NUM_CLKS];
	struct clk_hw_onecell_data *onecell;
};

#define to_bcmring_clk(_hw) container_of(_hw, struct bcmring_clk, hw)

static void chipc_rmw(struct bcmring_chipc *chipc, u32 reg, u32 clr, u32 set)
{
	regmap_update_bits(chipc->regmap, reg, clr | set, set);
}

static u32 chipc_read(struct bcmring_chipc *chipc, u32 reg)
{
	unsigned int val = 0;

	regmap_read(chipc->regmap, reg, &val);
	return val;
}

static unsigned int chipc_div(u32 field)
{
	return field ? field : 256;
}

/* ---- PLL VCOs (read-only; programmed by boot1/boot2) ------------------ */

static unsigned long bcmring_pll_recalc_rate(struct clk_hw *hw,
					     unsigned long parent_rate)
{
	struct bcmring_clk *clk = to_bcmring_clk(hw);
	u32 val = chipc_read(clk->chipc, clk->reg);
	u32 mode = FIELD_GET(PLL_PREDIV_MODE_MASK, val);
	u64 rate, frac;

	/*
	 * VCO = xtal * (P1 / P2) * (NDIV_int + NDIV_frac / 2^24), with
	 * P1 = P2 = 1 (PREDIV bits 3:0 and 7:4; both read 1 on a BCM11189).
	 */
	rate = (u64)parent_rate * FIELD_GET(PLL_PREDIV_NDIV_MASK, val);

	/*
	 * PLL1 can run in a sigma-delta (spread-spectrum) mode with a
	 * fractional feedback divider in the low 24 bits of PLL_DIVIDER.
	 * The old vendor source meant to add that term but its integer
	 * arithmetic drops it.  It is not negligible: a BCM11189 under the
	 * stock boot loader has NDIV_int 79, mode 1, and fraction 0xff0000,
	 * i.e., 1999.9 MHz rather than 1975 MHz, and only the former agrees
	 * with the UART divisor the boot loader programs for 115200 baud
	 * (77.5 at 2 GHz / 14).  With spread spectrum on, this is the top of
	 * the modulated range.
	 *
	 * The "1/8" modes are taken to scale the fraction by 1/8, going by
	 * their names.  PLL2's fraction register, if it has one, is not
	 * known; the boot loader leaves PLL2 in integer mode.
	 */
	if (clk->reg == CHIPC_PLL_PREDIV && mode) {
		frac = FIELD_GET(PLL_DIVIDER_FRAC_MASK,
				 chipc_read(clk->chipc, CHIPC_PLL_DIVIDER));
		frac *= parent_rate;
		if (mode == PLL_PREDIV_MODE_MASH_1_8 ||
		    mode == PLL_PREDIV_MODE_MFB_1_8)
			frac >>= 3;
		rate += frac >> 24;
	}

	return rate;
}

static const struct clk_ops bcmring_pll_ops = {
	.recalc_rate = bcmring_pll_recalc_rate,
};

/* ---- gate helpers shared by PLL channels and divider clocks ----------- */

static int bcmring_gate_enable(struct clk_hw *hw)
{
	struct bcmring_clk *clk = to_bcmring_clk(hw);

	if (clk->quirks & Q_PWRDN)
		chipc_rmw(clk->chipc, clk->reg, CLK_POWER_DOWN, 0);
	chipc_rmw(clk->chipc, clk->reg, 0, CLK_OUTPUT_ENABLE);
	return 0;
}

static void bcmring_gate_disable(struct clk_hw *hw)
{
	struct bcmring_clk *clk = to_bcmring_clk(hw);

	chipc_rmw(clk->chipc, clk->reg, CLK_OUTPUT_ENABLE, 0);
	if (clk->quirks & Q_PWRDN)
		chipc_rmw(clk->chipc, clk->reg, 0, CLK_POWER_DOWN);
}

static int bcmring_gate_is_enabled(struct clk_hw *hw)
{
	struct bcmring_clk *clk = to_bcmring_clk(hw);
	u32 val = chipc_read(clk->chipc, clk->reg);

	if ((clk->quirks & Q_PWRDN) && (val & CLK_POWER_DOWN))
		return 0;
	return !!(val & CLK_OUTPUT_ENABLE);
}

/* ---- PLL channels ------------------------------------------------------ */

static unsigned long bcmring_pllch_parent_rate(struct bcmring_clk *clk,
					       unsigned long parent_rate)
{
	if ((clk->quirks & Q_LCD) && !clk->chipc->rev_a0)
		parent_rate >>= 1;
	return parent_rate;
}

static unsigned long bcmring_pllch_recalc_rate(struct clk_hw *hw,
					       unsigned long parent_rate)
{
	struct bcmring_clk *clk = to_bcmring_clk(hw);
	struct bcmring_chipc *chipc = clk->chipc;
	u32 val = chipc_read(chipc, clk->reg);
	u32 div;

	if (val & CLK_BYPASS)
		return chipc->xtal_rate;

	if (clk->quirks & Q_DDR)
		div = FIELD_GET(PLL_DIVIDER_M1DIV_MASK,
				chipc_read(chipc, CHIPC_PLL_DIVIDER));
	else
		div = FIELD_GET(CLK_DIV_MASK, val);

	return bcmring_pllch_parent_rate(clk, parent_rate) / chipc_div(div);
}

/*
 * Nearest divider.  The vendor code truncates (MDIV = vco / rate), which
 * works for it only because it believes PLL1 runs at an integer multiple of
 * the crystal.  With the real fractional rate (1999.9 MHz on a BCM11189)
 * truncation would turn a request for 100 MHz into divider 19 (105.3 MHz)
 * instead of the 20 the boot loader programs, and 50 MHz into 39 instead
 * of 40.  The reachable range is parent / 256 .. parent; for the LCD channel
 * on a rev B0 or later chip with PLL2 at 1.8 GHz that is about 3.5 MHz ..
 * 900 MHz, which covers the 8.6 MHz pixel clock the phones use with the
 * PL111 divider bypassed (divider 104, as the stock firmware programs).
 */
static unsigned long bcmring_pllch_div(unsigned long parent, unsigned long rate)
{
	return clamp(DIV_ROUND_CLOSEST(parent, rate), 1UL, 256UL);
}

static int bcmring_pllch_determine_rate(struct clk_hw *hw,
					struct clk_rate_request *req)
{
	struct bcmring_clk *clk = to_bcmring_clk(hw);
	unsigned long parent;

	parent = bcmring_pllch_parent_rate(clk, req->best_parent_rate);
	if (!req->rate || !parent)
		return -EINVAL;

	req->rate = parent / bcmring_pllch_div(parent, req->rate);
	return 0;
}

static int bcmring_pllch_set_rate(struct clk_hw *hw, unsigned long rate,
				  unsigned long parent_rate)
{
	struct bcmring_clk *clk = to_bcmring_clk(hw);
	struct bcmring_chipc *chipc = clk->chipc;
	unsigned long parent, div;
	u32 val;

	parent = bcmring_pllch_parent_rate(clk, parent_rate);
	if (!rate || !parent)
		return -EINVAL;
	div = bcmring_pllch_div(parent, rate);

	/*
	 * Nothing to do if the channel already runs from the PLL with this
	 * divider (typical for rates the boot loader has set up): do not
	 * put a running consumer on the crystal for no reason.
	 */
	val = chipc_read(chipc, clk->reg);
	if (!(val & (CLK_BYPASS | CLK_SOURCE_GPIO)) &&
	    FIELD_GET(CLK_DIV_MASK, val) == (div & 0xff))
		return 0;

	/*
	 * Vendor sequence (chipcHw_setClockFrequency): select the crystal as
	 * bypass source, bypass, change MDIV, wait at least 200 ns, leave
	 * bypass.  Each step is an atomic read-modify-write, so a concurrent
	 * gate enable/disable of the same clock (which runs under the clock
	 * framework's enable lock, not its prepare lock) is not lost.
	 */
	chipc_rmw(chipc, clk->reg, CLK_SOURCE_GPIO, 0);
	chipc_rmw(chipc, clk->reg, 0, CLK_BYPASS);
	chipc_rmw(chipc, clk->reg, CLK_DIV_MASK,
		  FIELD_PREP(CLK_DIV_MASK, div & 0xff));	/* 256 -> 0 */
	udelay(1);
	chipc_rmw(chipc, clk->reg, CLK_BYPASS, 0);

	return 0;
}

static const struct clk_ops bcmring_pllch_ops = {
	.enable = bcmring_gate_enable,
	.disable = bcmring_gate_disable,
	.is_enabled = bcmring_gate_is_enabled,
	.recalc_rate = bcmring_pllch_recalc_rate,
	.determine_rate = bcmring_pllch_determine_rate,
	.set_rate = bcmring_pllch_set_rate,
};

/*
 * DDR, ARM, and VPM: nothing but the rate is exposed.  No rate changes: the
 * vendor code also rewrites the clock-to-bus ratio field (bits 21:20) for
 * DDR and VPM and runs a phase-alignment procedure afterwards.  No gate
 * operations either: on a running BCM11189 the DDR register reads
 * 0x00913200 (bit 16 set) and the VPM register 0x00103c06 (bit 16 clear)
 * while both blocks work, so bit 16 does not have the plain "output enable"
 * meaning there, and the old source's "DDR enable is inverted" would have
 * made the clock framework clear bit 16 of the DDR register when it enables
 * this critical clock.  These registers are never written.
 */
static const struct clk_ops bcmring_pllch_ro_ops = {
	.recalc_rate = bcmring_pllch_recalc_rate,
};

/* ---- divider clocks ---------------------------------------------------- */

static unsigned long bcmring_div_recalc_rate(struct clk_hw *hw,
					     unsigned long parent_rate)
{
	struct bcmring_clk *clk = to_bcmring_clk(hw);
	u32 val = chipc_read(clk->chipc, clk->reg);

	if (val & CLK_BYPASS)
		return clk->chipc->xtal_rate;

	return parent_rate / chipc_div(FIELD_GET(CLK_DIV_MASK, val));
}

static int bcmring_div_determine_rate(struct clk_hw *hw,
				      struct clk_rate_request *req)
{
	unsigned long div;

	if (!req->rate || !req->best_parent_rate)
		return -EINVAL;

	div = clamp(DIV_ROUND_CLOSEST(req->best_parent_rate, req->rate),
		    1UL, 256UL);
	req->rate = req->best_parent_rate / div;
	return 0;
}

static int bcmring_div_set_rate(struct clk_hw *hw, unsigned long rate,
				unsigned long parent_rate)
{
	struct bcmring_clk *clk = to_bcmring_clk(hw);
	unsigned long div;

	if (!rate || !parent_rate)
		return -EINVAL;
	div = clamp(DIV_ROUND_CLOSEST(parent_rate, rate), 1UL, 256UL);

	/* Vendor order: leave bypass first, then program the divider. */
	chipc_rmw(clk->chipc, clk->reg, CLK_BYPASS, 0);
	chipc_rmw(clk->chipc, clk->reg, CLK_DIV_MASK,
		  FIELD_PREP(CLK_DIV_MASK, div & 0xff));
	return 0;
}

static const struct clk_ops bcmring_div_ops = {
	.enable = bcmring_gate_enable,
	.disable = bcmring_gate_disable,
	.is_enabled = bcmring_gate_is_enabled,
	.recalc_rate = bcmring_div_recalc_rate,
	.determine_rate = bcmring_div_determine_rate,
	.set_rate = bcmring_div_set_rate,
};

/* Bus clocks: always on, never written. */
static const struct clk_ops bcmring_div_ro_ops = {
	.recalc_rate = bcmring_div_recalc_rate,
};

/* ---- bus-interface gates ----------------------------------------------- */

static int bcmring_busif_enable(struct clk_hw *hw)
{
	struct bcmring_clk *clk = to_bcmring_clk(hw);

	chipc_rmw(clk->chipc, CHIPC_BUSINTF_CLK, 0, BIT(clk->bit));
	return 0;
}

static void bcmring_busif_disable(struct clk_hw *hw)
{
	struct bcmring_clk *clk = to_bcmring_clk(hw);

	chipc_rmw(clk->chipc, CHIPC_BUSINTF_CLK, BIT(clk->bit), 0);
}

static int bcmring_busif_is_enabled(struct clk_hw *hw)
{
	struct bcmring_clk *clk = to_bcmring_clk(hw);

	return !!(chipc_read(clk->chipc, CHIPC_BUSINTF_CLK) & BIT(clk->bit));
}

static const struct clk_ops bcmring_busif_ops = {
	.enable = bcmring_busif_enable,
	.disable = bcmring_busif_disable,
	.is_enabled = bcmring_busif_is_enabled,
};

/* ---- clock table ------------------------------------------------------- */

struct bcmring_clk_desc {
	const char *name;
	u8 id;
	u8 kind;
	u8 parent;	/* clock ID or PARENT_XTAL */
	u8 quirks;
	u16 reg;
	bool critical;
};

/* Parents must be listed before their children. */
static const struct bcmring_clk_desc bcmring_clk_descs[] = {
	{ "pll1", BCMRING_CLK_PLL1, KIND_PLL, PARENT_XTAL, 0, CHIPC_PLL_PREDIV },
	{ "pll2", BCMRING_CLK_PLL2, KIND_PLL, PARENT_XTAL, 0, CHIPC_PLL_PREDIV2 },

	{ "ddr", BCMRING_CLK_DDR, KIND_PLLCH, BCMRING_CLK_PLL1, Q_RO | Q_DDR, CHIPC_DDR_CLK, true },
	{ "arm", BCMRING_CLK_ARM, KIND_PLLCH, BCMRING_CLK_PLL1, Q_RO, CHIPC_ARM_CLK, true },
	{ "esw", BCMRING_CLK_ESW, KIND_PLLCH, BCMRING_CLK_PLL1, 0, CHIPC_ESW_CLK },
	{ "vpm", BCMRING_CLK_VPM, KIND_PLLCH, BCMRING_CLK_PLL1, Q_RO, CHIPC_VPM_CLK },
	{ "esw125", BCMRING_CLK_ESW125, KIND_PLLCH, BCMRING_CLK_PLL1, 0, CHIPC_ESW125_CLK },
	{ "uart", BCMRING_CLK_UART, KIND_PLLCH, BCMRING_CLK_PLL1, 0, CHIPC_UART_CLK },
	{ "sdio0", BCMRING_CLK_SDIO0, KIND_PLLCH, BCMRING_CLK_PLL1, 0, CHIPC_SDIO0_CLK },
	{ "sdio1", BCMRING_CLK_SDIO1, KIND_PLLCH, BCMRING_CLK_PLL1, 0, CHIPC_SDIO1_CLK },
	{ "spi", BCMRING_CLK_SPI, KIND_PLLCH, BCMRING_CLK_PLL1, 0, CHIPC_SPI_CLK },
	{ "etm", BCMRING_CLK_ETM, KIND_PLLCH, BCMRING_CLK_PLL1, 0, CHIPC_ETM_CLK },
	{ "usb", BCMRING_CLK_USB, KIND_PLLCH, BCMRING_CLK_PLL2, Q_PWRDN, CHIPC_USB_CLK },
	{ "lcd", BCMRING_CLK_LCD, KIND_PLLCH, BCMRING_CLK_PLL2, Q_PWRDN | Q_LCD, CHIPC_LCD_CLK },
	{ "apm200", BCMRING_CLK_APM, KIND_PLLCH, BCMRING_CLK_PLL2, Q_PWRDN, CHIPC_APM_CLK },

	{ "bus", BCMRING_CLK_BUS, KIND_DIV, BCMRING_CLK_ARM, Q_RO, CHIPC_ACLK_CLK, true },
	{ "rtbus", BCMRING_CLK_RTBUS, KIND_DIV, BCMRING_CLK_BUS, Q_RO, CHIPC_RTBUS_CLK, true },
	{ "otp", BCMRING_CLK_OTP, KIND_DIV, PARENT_XTAL, 0, CHIPC_OTP_CLK },
	{ "i2c", BCMRING_CLK_I2C, KIND_DIV, PARENT_XTAL, 0, CHIPC_I2C_CLK },
	/*
	 * The vendor Linux clock table names apm100 as the parent of i2s0 and
	 * i2s1, but chipcHw_{get,set}ClockFrequency() compute both from the
	 * crystal.  The register-level code is followed here.
	 */
	{ "i2s0", BCMRING_CLK_I2S0, KIND_DIV, PARENT_XTAL, 0, CHIPC_I2S0_CLK },
	{ "i2s1", BCMRING_CLK_I2S1, KIND_DIV, PARENT_XTAL, 0, CHIPC_I2S1_CLK },
	{ "apm100", BCMRING_CLK_APM100, KIND_DIV, BCMRING_CLK_APM, 0, CHIPC_APM100_CLK },
	{ "tsc", BCMRING_CLK_TSC, KIND_DIV, PARENT_XTAL, 0, CHIPC_TSC_CLK },
	{ "led", BCMRING_CLK_LED, KIND_DIV, PARENT_XTAL, 0, CHIPC_LED_CLK },
};

static const char * const bcmring_busif_names[32] = {
	"busif-arm", "busif-vdec", "busif-aram", "busif-hpm",
	"busif-ddrc", "busif-dmac0", "busif-dmac1", "busif-nvi",
	"busif-esw", "busif-ge", "busif-i2ch", "busif-i2s0",
	"busif-i2s1", "busif-vram", "busif-clcd", "busif-ldk",
	"busif-led", "busif-otp", "busif-pif", "busif-spu",
	"busif-sdio0", "busif-sdio1", "busif-spih", "busif-spis",
	"busif-uart0", "busif-uart1", "busif-bbl", "busif-i2cs",
	"busif-usbh", "busif-usbd", "busif-brom", "busif-tsc",
};

static int bcmring_register_clk(struct device *dev, struct bcmring_chipc *chipc,
				const char *name, unsigned int id, u8 kind,
				u8 parent, u8 quirks, u16 reg, u8 bit,
				bool critical)
{
	struct bcmring_clk *clk = &chipc->clks[id];
	struct clk_parent_data pdata = { };
	struct clk_init_data init = { };
	int ret;

	if (parent == PARENT_XTAL)
		pdata.fw_name = "xtal";
	else
		pdata.hw = &chipc->clks[parent].hw;

	clk->chipc = chipc;
	clk->reg = reg;
	clk->kind = kind;
	clk->bit = bit;
	clk->quirks = quirks;

	switch (kind) {
	case KIND_PLL:
		init.ops = &bcmring_pll_ops;
		break;
	case KIND_PLLCH:
		init.ops = (quirks & Q_RO) ? &bcmring_pllch_ro_ops :
					     &bcmring_pllch_ops;
		break;
	case KIND_DIV:
		init.ops = (quirks & Q_RO) ? &bcmring_div_ro_ops :
					     &bcmring_div_ops;
		break;
	default:
		init.ops = &bcmring_busif_ops;
		break;
	}

	init.name = name;
	init.parent_data = &pdata;
	init.num_parents = 1;
	/*
	 * CLK_IGNORE_UNUSED is set on everything, because it is not known
	 * which of the gates left on by boot2 the system depends on (ARAM,
	 * VRAM, HPM, DDRC, BROM, ...).
	 */
	init.flags = CLK_IGNORE_UNUSED;
	if (critical)
		init.flags |= CLK_IS_CRITICAL;

	clk->hw.init = &init;
	ret = devm_clk_hw_register(dev, &clk->hw);
	if (ret)
		return dev_err_probe(dev, ret, "failed to register %s\n", name);

	chipc->onecell->hws[id] = &clk->hw;
	return 0;
}

/* ---- reset controller -------------------------------------------------- */

#define rcdev_to_chipc(_r) container_of(_r, struct bcmring_chipc, rcdev)

static int bcmring_reset_update(struct reset_controller_dev *rcdev,
				unsigned long id, bool assert)
{
	struct bcmring_chipc *chipc = rcdev_to_chipc(rcdev);
	u32 reg = id < 32 ? CHIPC_SOFT_RESET1 : CHIPC_SOFT_RESET2;
	u32 bit = BIT(id % 32);

	/* chip/ARM resets and the unhold request bits are not reset lines */
	if (id < BCMRING_RST_ETM || id >= BCMRING_NUM_RESETS)
		return -EINVAL;

	/* A set bit holds the block in reset. */
	chipc_rmw(chipc, reg, assert ? 0 : bit, assert ? bit : 0);
	return 0;
}

static int bcmring_reset_assert(struct reset_controller_dev *rcdev,
				unsigned long id)
{
	return bcmring_reset_update(rcdev, id, true);
}

static int bcmring_reset_deassert(struct reset_controller_dev *rcdev,
				  unsigned long id)
{
	return bcmring_reset_update(rcdev, id, false);
}

static int bcmring_reset_status(struct reset_controller_dev *rcdev,
				unsigned long id)
{
	struct bcmring_chipc *chipc = rcdev_to_chipc(rcdev);
	u32 reg = id < 32 ? CHIPC_SOFT_RESET1 : CHIPC_SOFT_RESET2;

	if (id >= BCMRING_NUM_RESETS)
		return -EINVAL;

	return !!(chipc_read(chipc, reg) & BIT(id % 32));
}

static const struct reset_control_ops bcmring_reset_ops = {
	.assert = bcmring_reset_assert,
	.deassert = bcmring_reset_deassert,
	.status = bcmring_reset_status,
};

/* ---- probe ------------------------------------------------------------- */

static const struct regmap_config bcmring_chipc_regmap_config = {
	.name = "chipc",
	.reg_bits = 32,
	.val_bits = 32,
	.reg_stride = 4,
	/* spinlock instead of mutex: clock enable/disable run atomically */
	.fast_io = true,
};

static int bcmring_chipc_probe(struct platform_device *pdev)
{
	struct regmap_config regmap_config = bcmring_chipc_regmap_config;
	struct device *dev = &pdev->dev;
	struct bcmring_chipc *chipc;
	void __iomem *base;
	struct resource *res;
	struct clk *xtal;
	unsigned int i;
	u32 chipid;
	int ret;

	chipc = devm_kzalloc(dev, sizeof(*chipc), GFP_KERNEL);
	if (!chipc)
		return -ENOMEM;

	chipc->onecell = devm_kzalloc(dev, struct_size(chipc->onecell, hws,
						       BCMRING_NUM_CLKS),
				      GFP_KERNEL);
	if (!chipc->onecell)
		return -ENOMEM;
	chipc->onecell->num = BCMRING_NUM_CLKS;
	for (i = 0; i < BCMRING_NUM_CLKS; i++)
		chipc->onecell->hws[i] = ERR_PTR(-ENOENT);

	/*
	 * The region is not requested exclusively: the pinctrl-single child
	 * nodes request their own slices of it (0x114, 0x118..0x137), which
	 * nothing in this driver or any syscon user touches.
	 */
	res = platform_get_resource(pdev, IORESOURCE_MEM, 0);
	if (!res)
		return -EINVAL;
	base = devm_ioremap(dev, res->start, resource_size(res));
	if (!base)
		return -ENOMEM;

	regmap_config.max_register = resource_size(res) - 4;
	chipc->regmap = devm_regmap_init_mmio(dev, base, &regmap_config);
	if (IS_ERR(chipc->regmap))
		return dev_err_probe(dev, PTR_ERR(chipc->regmap),
				     "failed to create regmap\n");

	xtal = devm_clk_get(dev, "xtal");
	if (IS_ERR(xtal))
		return dev_err_probe(dev, PTR_ERR(xtal), "no xtal clock\n");
	chipc->xtal_rate = clk_get_rate(xtal);

	chipid = chipc_read(chipc, CHIPC_CHIPID);
	chipc->rev_a0 = (chipid & CHIPID_REV_MASK) == CHIPID_REV_A0;
	dev_info(dev, "chip ID register %#010x, xtal %lu Hz\n", chipid,
		 chipc->xtal_rate);

	for (i = 0; i < ARRAY_SIZE(bcmring_clk_descs); i++) {
		const struct bcmring_clk_desc *d = &bcmring_clk_descs[i];

		ret = bcmring_register_clk(dev, chipc, d->name, d->id, d->kind,
					   d->parent, d->quirks, d->reg, 0,
					   d->critical);
		if (ret)
			return ret;
	}

	for (i = 0; i < 32; i++) {
		ret = bcmring_register_clk(dev, chipc, bcmring_busif_names[i],
					   BCMRING_CLK_BUSIF(i), KIND_BUSIF,
					   BCMRING_CLK_BUS, 0,
					   CHIPC_BUSINTF_CLK, i, false);
		if (ret)
			return ret;
	}

	ret = devm_of_clk_add_hw_provider(dev, of_clk_hw_onecell_get,
					  chipc->onecell);
	if (ret)
		return ret;

	chipc->rcdev.owner = THIS_MODULE;
	chipc->rcdev.ops = &bcmring_reset_ops;
	chipc->rcdev.of_node = dev->of_node;
	chipc->rcdev.nr_resets = BCMRING_NUM_RESETS;

	ret = devm_reset_controller_register(dev, &chipc->rcdev);
	if (ret)
		return ret;

	/* Pin multiplexing (pinctrl-single) lives in child nodes. */
	ret = devm_of_platform_populate(dev);
	if (ret)
		return ret;

	/*
	 * Last, because the syscon list entry is not device-managed: nothing
	 * may fail after this point.  Consumers that look the regmap up
	 * earlier get -EPROBE_DEFER.
	 */
	ret = of_syscon_register_regmap(dev->of_node, chipc->regmap);
	if (ret)
		return dev_err_probe(dev, ret,
				     "failed to register syscon regmap\n");

	return 0;
}

static const struct of_device_id bcmring_chipc_of_match[] = {
	{ .compatible = "brcm,bcmring-chipc" },
	{ }
};
MODULE_DEVICE_TABLE(of, bcmring_chipc_of_match);

static struct platform_driver bcmring_chipc_driver = {
	.probe = bcmring_chipc_probe,
	.driver = {
		.name = "bcmring-chipc",
		.of_match_table = bcmring_chipc_of_match,
		.suppress_bind_attrs = true,
	},
};

static int __init bcmring_chipc_init(void)
{
	return platform_driver_register(&bcmring_chipc_driver);
}
core_initcall(bcmring_chipc_init);

MODULE_DESCRIPTION("Broadcom BCMRING chip-control clock and reset driver");
MODULE_LICENSE("GPL");
