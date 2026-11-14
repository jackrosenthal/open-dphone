// SPDX-License-Identifier: GPL-2.0-only
/*
 * LED matrix controller ("LEDM") of Broadcom BCMRING SoCs (BCM11107
 * family), as used by the Digium/Sangoma D40/D45/D50/D70/D60/D62/D65.
 *
 * FIRST DRAFT.  Compile-tested only; never run on hardware.
 *
 * New code written from behavior recovered from the vendor kernels
 * (ledm_probe, ledm_brightness_set, led_clk_set, ledmHw_Init,
 * ledmHw_Enable, ledmHw_WriteCoords in the 2.6.27.18 and 3.4.74 images;
 * identical in both families).  The driver was never in mainline; the
 * block address matches MM_ADDR_IO_LEDM in the old mainline
 * arch/arm/mach-bcmring/include/mach/csp/mm_addr.h.  Nothing was copied.
 * See NOTES.md.
 *
 * Hardware summary (verified from the binaries; field names follow the
 * vendor's boot message "mode=%s clk_div=%u res_div=%u on_time=%u
 * guard_time=%u"):
 *   base 0x80002000 (the PWM block used for the LCD backlight follows at
 *   0x80002040 and is not touched here)
 *   0x00 control
 *        3:0   number of scanned columns (1..12)
 *        6:4   number of scanned rows (1..6)
 *        15:8  res_div
 *        16    enable
 *        17    flag, vendor platform data sets it to 1 (meaning unknown)
 *        18    flag, vendor platform data sets it to 0 (meaning unknown)
 *        20:19 clk_div (0..3)
 *        21    1 = serial mode (pads 16/17, mux function 6),
 *              0 = parallel mode (one pad per row and column, function 5)
 *        31    written as 1 after every data change ("update")
 *   0x04 on_time
 *   0x08 guard_time
 *   0x18, 0x1c, 0x20  lamp data, two 12-bit fields per register:
 *        hardware row r (0..5) lives in register 0x18 + 4 * (r / 2),
 *        bits 11:0 for even r and bits 27:16 for odd r; bit c = column c.
 *        1 = lit.
 *
 * The controller multiplexes and refreshes by itself; software only sets
 * or clears one bit per lamp.  There is no per-lamp brightness.
 */

#include <linux/bitfield.h>
#include <linux/bitops.h>
#include <linux/clk.h>
#include <linux/io.h>
#include <linux/leds.h>
#include <linux/mod_devicetable.h>
#include <linux/module.h>
#include <linux/platform_device.h>
#include <linux/property.h>
#include <linux/spinlock.h>

#define LEDM_CTRL		0x00
#define LEDM_CTRL_COLS		GENMASK(3, 0)
#define LEDM_CTRL_ROWS		GENMASK(6, 4)
#define LEDM_CTRL_RES_DIV	GENMASK(15, 8)
#define LEDM_CTRL_ENABLE	BIT(16)
#define LEDM_CTRL_FLAG17	BIT(17)	/* vendor: 1 */
#define LEDM_CTRL_FLAG18	BIT(18)	/* vendor: 0 */
#define LEDM_CTRL_CLK_DIV	GENMASK(20, 19)
#define LEDM_CTRL_SERIAL	BIT(21)
#define LEDM_CTRL_UPDATE	BIT(31)
#define LEDM_ON_TIME		0x04
#define LEDM_GUARD_TIME		0x08
#define LEDM_DATA(row)		(0x18 + 4 * ((row) / 2))
#define LEDM_DATA_SHIFT(row)	(((row) & 1) ? 16 : 0)
#define LEDM_DATA_FIELD		GENMASK(11, 0)

#define LEDM_MAX_ROWS		6
#define LEDM_MAX_COLS		12

struct bcmring_ledm;

struct bcmring_ledm_led {
	struct led_classdev cdev;
	struct bcmring_ledm *ledm;
	u8 row;
	u8 col;
};

struct bcmring_ledm {
	void __iomem *base;
	spinlock_t lock;
	struct clk *bus_clk;
	struct clk *led_clk;
};

static void bcmring_ledm_set(struct led_classdev *cdev,
			     enum led_brightness brightness)
{
	struct bcmring_ledm_led *led =
		container_of(cdev, struct bcmring_ledm_led, cdev);
	struct bcmring_ledm *ledm = led->ledm;
	u32 bit = BIT(led->col) << LEDM_DATA_SHIFT(led->row);
	unsigned long flags;
	u32 val;

	spin_lock_irqsave(&ledm->lock, flags);

	val = readl(ledm->base + LEDM_DATA(led->row));
	if (brightness)
		val |= bit;
	else
		val &= ~bit;
	writel(val, ledm->base + LEDM_DATA(led->row));

	/* ledmHw_WriteCoords() ends every change with this. */
	val = readl(ledm->base + LEDM_CTRL);
	writel(val | LEDM_CTRL_UPDATE, ledm->base + LEDM_CTRL);

	spin_unlock_irqrestore(&ledm->lock, flags);
}

static void bcmring_ledm_disable(void *data)
{
	struct bcmring_ledm *ledm = data;
	u32 val;

	val = readl(ledm->base + LEDM_CTRL);
	writel(val & ~LEDM_CTRL_ENABLE, ledm->base + LEDM_CTRL);
}

static void bcmring_ledm_hw_init(struct bcmring_ledm *ledm, struct device *dev)
{
	/*
	 * Defaults are the values compiled into the D60/D62/D65 board file
	 * where they agree across all seven models; on_time and guard_time
	 * differ between the two kernel families and should come from the
	 * devicetree.
	 */
	u32 scan_rows = 5, scan_cols = 8;	/* hard-coded for parallel mode */
	u32 clk_div = 2, res_div = 100;
	u32 on_time = 0x100, guard_time = 100;
	unsigned int row;
	u32 ctrl;

	device_property_read_u32(dev, "brcm,scan-rows", &scan_rows);
	device_property_read_u32(dev, "brcm,scan-columns", &scan_cols);
	device_property_read_u32(dev, "brcm,clk-div", &clk_div);
	device_property_read_u32(dev, "brcm,res-div", &res_div);
	device_property_read_u32(dev, "brcm,on-time", &on_time);
	device_property_read_u32(dev, "brcm,guard-time", &guard_time);

	scan_rows = clamp_t(u32, scan_rows, 1, LEDM_MAX_ROWS);
	scan_cols = clamp_t(u32, scan_cols, 1, LEDM_MAX_COLS);

	/* Order follows ledm_probe(). */
	ctrl = readl(ledm->base + LEDM_CTRL);
	ctrl &= ~LEDM_CTRL_CLK_DIV;
	ctrl |= FIELD_PREP(LEDM_CTRL_CLK_DIV, clk_div);
	writel(ctrl, ledm->base + LEDM_CTRL);

	ctrl &= ~LEDM_CTRL_RES_DIV;
	ctrl |= FIELD_PREP(LEDM_CTRL_RES_DIV, res_div);
	writel(ctrl, ledm->base + LEDM_CTRL);

	writel(on_time, ledm->base + LEDM_ON_TIME);
	writel(guard_time, ledm->base + LEDM_GUARD_TIME);

	/* All lamps off; the vendor clears the fields, not the registers. */
	for (row = 0; row < LEDM_MAX_ROWS; row++) {
		u32 val = readl(ledm->base + LEDM_DATA(row));

		val &= ~(LEDM_DATA_FIELD << LEDM_DATA_SHIFT(row));
		writel(val, ledm->base + LEDM_DATA(row));
	}

	/* ledmHw_Init(serial = 0, rows, cols, flag18 = 0, flag17 = 1). */
	ctrl = readl(ledm->base + LEDM_CTRL);
	ctrl &= ~(LEDM_CTRL_FLAG18 | LEDM_CTRL_ROWS | LEDM_CTRL_COLS |
		  LEDM_CTRL_SERIAL);
	ctrl |= LEDM_CTRL_FLAG17;
	ctrl |= FIELD_PREP(LEDM_CTRL_ROWS, scan_rows);
	ctrl |= FIELD_PREP(LEDM_CTRL_COLS, scan_cols);
	/*
	 * TODO: serial mode (bit 21, pads 16/17 on mux function 6) exists in
	 * the vendor driver but no Digium board selects it; not implemented.
	 */
	writel(ctrl, ledm->base + LEDM_CTRL);
	writel(ctrl | LEDM_CTRL_ENABLE, ledm->base + LEDM_CTRL);
}

static int bcmring_ledm_probe(struct platform_device *pdev)
{
	struct device *dev = &pdev->dev;
	struct bcmring_ledm *ledm;
	int ret;

	ledm = devm_kzalloc(dev, sizeof(*ledm), GFP_KERNEL);
	if (!ledm)
		return -ENOMEM;

	ledm->base = devm_platform_ioremap_resource(pdev, 0);
	if (IS_ERR(ledm->base))
		return PTR_ERR(ledm->base);

	spin_lock_init(&ledm->lock);

	/*
	 * "bus" is the LED bus-interface gate (chip control BusIntfClock,
	 * offset 0x5c, bit 16), shared with the PWM block.  "led" is the
	 * vendor's "LED_PWM" clock (chip control LEDClock divider, 0x4c).
	 */
	ledm->bus_clk = devm_clk_get_enabled(dev, "bus");
	if (IS_ERR(ledm->bus_clk))
		return dev_err_probe(dev, PTR_ERR(ledm->bus_clk),
				     "no bus clock\n");

	ledm->led_clk = devm_clk_get_enabled(dev, "led");
	if (IS_ERR(ledm->led_clk))
		return dev_err_probe(dev, PTR_ERR(ledm->led_clk),
				     "no LED clock\n");

	bcmring_ledm_hw_init(ledm, dev);

	ret = devm_add_action_or_reset(dev, bcmring_ledm_disable, ledm);
	if (ret)
		return ret;

	device_for_each_child_node_scoped(dev, child) {
		struct led_init_data init_data = { .fwnode = child };
		struct bcmring_ledm_led *led;
		u32 reg[2];

		ret = fwnode_property_read_u32_array(child, "reg", reg, 2);
		if (ret)
			return dev_err_probe(dev, ret, "LED without reg\n");
		if (reg[0] >= LEDM_MAX_ROWS || reg[1] >= LEDM_MAX_COLS)
			return dev_err_probe(dev, -EINVAL,
					     "LED position %u,%u out of range\n",
					     reg[0], reg[1]);

		led = devm_kzalloc(dev, sizeof(*led), GFP_KERNEL);
		if (!led)
			return -ENOMEM;

		led->ledm = ledm;
		led->row = reg[0];
		led->col = reg[1];
		led->cdev.max_brightness = 1;
		led->cdev.brightness_set = bcmring_ledm_set;

		ret = devm_led_classdev_register_ext(dev, &led->cdev,
						     &init_data);
		if (ret)
			return dev_err_probe(dev, ret,
					     "failed to register LED %u,%u\n",
					     reg[0], reg[1]);
	}

	return 0;
}

static const struct of_device_id bcmring_ledm_of_match[] = {
	{ .compatible = "brcm,bcmring-ledm" },
	{ }
};
MODULE_DEVICE_TABLE(of, bcmring_ledm_of_match);

static struct platform_driver bcmring_ledm_driver = {
	.probe = bcmring_ledm_probe,
	.driver = {
		.name = "leds-bcmring-ledm",
		.of_match_table = bcmring_ledm_of_match,
	},
};
module_platform_driver(bcmring_ledm_driver);

MODULE_DESCRIPTION("Broadcom BCMRING LED matrix driver (first draft)");
MODULE_LICENSE("GPL");
