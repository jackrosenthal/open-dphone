// SPDX-License-Identifier: GPL-2.0-only
/*
 * Keypad matrix controller ("KEYC") of Broadcom BCMRING SoCs (BCM11107
 * family), as used by the Digium/Sangoma D40/D45/D50/D70/D60/D62/D65.
 *
 * FIRST DRAFT.  Compile-tested only; never run on hardware.
 *
 * New code written from behavior recovered from the vendor kernels
 * (keypad_probe, keypad_interrupt, keypad_scan, gpio_keyreserve,
 * keypad_suspend/resume in the 2.6.27.18 and 3.4.74 images; the two
 * families are instruction-for-instruction equivalent here).  The keypad
 * driver was never in mainline; nothing was copied.  See NOTES.md.
 *
 * Hardware summary (all verified from the binaries unless noted):
 *   base 0x8000d000, IRQ_KEYC = 48 (INTC1 bit 16)
 *   0x00..0x10  five state registers.  Register n holds two 9-bit line
 *               groups: bits 24:16 = scan-code row 2n, bits 8:0 = row
 *               2n+1.  A 0 bit means "pressed".
 *   0x14        16-bit prescaler: clk / (prescale + 1) / 9 = sample rate
 *   0x18        8-bit debounce count (in samples)
 *   0x1c        control: bit 0 enable, bit 1 interrupt enable,
 *               bit 3 (vendor clears; meaning unknown),
 *               bit 4 (vendor sets; meaning unknown)
 *
 * Scan code = (row << 4) | column.  The vendor board code ties a scan code
 * to pads as: column c -> GPIO 43 + c, row r -> GPIO 52 + r, both muxed to
 * function 1 (KEYPAD).  Which side the controller drives and which it
 * senses is not recovered and does not matter to this driver.
 *
 * Not carried over from the vendor driver (policy, documented in NOTES.md):
 *   - the '7'+'5'+'3' held-for-5-s kernel_restart() combination;
 *   - the up/down versus left/right mutual lockout (is_key_allowed);
 *   - the hookswitch (GPIO 51) and optional extra GPIO keys, which belong
 *     to gpio-keys in the devicetree.
 */

#include <linux/bitfield.h>
#include <linux/bitops.h>
#include <linux/clk.h>
#include <linux/input.h>
#include <linux/input/matrix_keypad.h>
#include <linux/interrupt.h>
#include <linux/io.h>
#include <linux/log2.h>
#include <linux/mod_devicetable.h>
#include <linux/module.h>
#include <linux/platform_device.h>
#include <linux/pm.h>
#include <linux/property.h>

#define KEYC_STATE(n)		(0x00 + 4 * (n))
#define KEYC_NUM_STATE_REGS	5
#define KEYC_STATE_EVEN		GENMASK(24, 16)	/* scan row 2n */
#define KEYC_STATE_ODD		GENMASK(8, 0)	/* scan row 2n + 1 */
#define KEYC_PRESCALE		0x14
#define KEYC_PRESCALE_MASK	GENMASK(15, 0)
#define KEYC_DEBOUNCE		0x18
#define KEYC_DEBOUNCE_MASK	GENMASK(7, 0)
#define KEYC_CTRL		0x1c
#define KEYC_CTRL_ENABLE	BIT(0)
#define KEYC_CTRL_IRQ_ENABLE	BIT(1)
#define KEYC_CTRL_UNKNOWN3	BIT(3)	/* cleared by the vendor driver */
#define KEYC_CTRL_UNKNOWN4	BIT(4)	/* set by the vendor driver */

#define KEYC_MAX_ROWS		(2 * KEYC_NUM_STATE_REGS)
#define KEYC_MAX_COLS		9
/* The controller always cycles through nine lines. */
#define KEYC_SCAN_LINES		9

/* Vendor defaults: 9000 samples/s, debounce 200 samples (about 22 ms). */
#define KEYC_DEFAULT_SAMPLE_HZ	9000
#define KEYC_DEFAULT_DEBOUNCE	200
/* The vendor suspend handler drops the debounce count to this. */
#define KEYC_SUSPEND_DEBOUNCE	15

struct bcmring_keypad {
	struct device *dev;
	void __iomem *base;
	struct clk *clk;
	struct input_dev *input;
	unsigned int rows;
	unsigned int cols;
	unsigned int row_shift;
	u32 debounce;
	u16 last[KEYC_MAX_ROWS];	/* 1 = pressed */
};

static void bcmring_keypad_ctrl(struct bcmring_keypad *kp, u32 clear, u32 set)
{
	u32 val;

	val = readl(kp->base + KEYC_CTRL);
	val &= ~clear;
	val |= set;
	writel(val, kp->base + KEYC_CTRL);
}

static void bcmring_keypad_scan(struct bcmring_keypad *kp)
{
	const unsigned short *keycodes = kp->input->keycode;
	u16 now[KEYC_MAX_ROWS];
	unsigned int row, col;
	bool sync = false;
	u32 val;

	for (row = 0; row < KEYC_NUM_STATE_REGS; row++) {
		val = readl(kp->base + KEYC_STATE(row));
		/* Bits are active low. */
		now[2 * row] = ~FIELD_GET(KEYC_STATE_EVEN, val) & 0x1ff;
		now[2 * row + 1] = ~FIELD_GET(KEYC_STATE_ODD, val) & 0x1ff;
	}

	for (row = 0; row < kp->rows; row++) {
		unsigned long changed = now[row] ^ kp->last[row];

		for_each_set_bit(col, &changed, kp->cols) {
			unsigned int code = MATRIX_SCAN_CODE(row, col,
							     kp->row_shift);

			/* Like the vendor driver, ignore unmapped positions. */
			if (keycodes[code] == KEY_RESERVED)
				continue;

			/* Vendor scan code format: (row << 4) | column. */
			input_event(kp->input, EV_MSC, MSC_SCAN,
				    (row << 4) | col);
			input_report_key(kp->input, keycodes[code],
					 now[row] & BIT(col));
			sync = true;
		}
		kp->last[row] = now[row];
	}

	if (sync)
		input_sync(kp->input);
}

static irqreturn_t bcmring_keypad_irq(int irq, void *data)
{
	struct bcmring_keypad *kp = data;

	/*
	 * Vendor sequence: clear the interrupt-enable bit, read all five
	 * state registers, report, set the enable bit again.  Whether the
	 * request is dropped by the enable toggle or by the reads is not
	 * known, so keep exactly that order.
	 */
	bcmring_keypad_ctrl(kp, KEYC_CTRL_IRQ_ENABLE, 0);
	bcmring_keypad_scan(kp);
	bcmring_keypad_ctrl(kp, 0, KEYC_CTRL_IRQ_ENABLE);

	return IRQ_HANDLED;
}

static int bcmring_keypad_start(struct bcmring_keypad *kp)
{
	unsigned long rate;
	u32 prescale;
	int ret;

	/*
	 * In the vendor kernel this is two things: the "LDK" bus-interface
	 * gate (chip control BusIntfClock, offset 0x5c, bit 15) and the rate
	 * of the bus clock (chipcHw_CLOCK_BUS, id 10) that feeds the
	 * prescaler.  The clock given in the devicetree must be that gate,
	 * parented to the bus clock.
	 */
	ret = clk_prepare_enable(kp->clk);
	if (ret)
		return ret;

	rate = clk_get_rate(kp->clk);
	if (!rate) {
		clk_disable_unprepare(kp->clk);
		return dev_err_probe(kp->dev, -EINVAL, "clock rate unknown\n");
	}

	/* Same arithmetic as the vendor probe: rate / 9000 / 9 - 1. */
	prescale = rate / KEYC_DEFAULT_SAMPLE_HZ / KEYC_SCAN_LINES;
	prescale = clamp_t(u32, prescale, 1, KEYC_PRESCALE_MASK + 1) - 1;

	bcmring_keypad_ctrl(kp, KEYC_CTRL_ENABLE, 0);
	bcmring_keypad_ctrl(kp, KEYC_CTRL_UNKNOWN3, 0);
	bcmring_keypad_ctrl(kp, 0, KEYC_CTRL_UNKNOWN4);
	writel(kp->debounce, kp->base + KEYC_DEBOUNCE);
	writel(prescale, kp->base + KEYC_PRESCALE);
	bcmring_keypad_ctrl(kp, 0, KEYC_CTRL_ENABLE);
	bcmring_keypad_ctrl(kp, 0, KEYC_CTRL_IRQ_ENABLE);

	return 0;
}

static void bcmring_keypad_stop(struct bcmring_keypad *kp)
{
	bcmring_keypad_ctrl(kp, KEYC_CTRL_IRQ_ENABLE, 0);
	bcmring_keypad_ctrl(kp, KEYC_CTRL_ENABLE, 0);
	clk_disable_unprepare(kp->clk);
}

static int bcmring_keypad_open(struct input_dev *input)
{
	return bcmring_keypad_start(input_get_drvdata(input));
}

static void bcmring_keypad_close(struct input_dev *input)
{
	bcmring_keypad_stop(input_get_drvdata(input));
}

static int bcmring_keypad_probe(struct platform_device *pdev)
{
	struct device *dev = &pdev->dev;
	struct bcmring_keypad *kp;
	struct input_dev *input;
	u32 debounce_ms;
	int irq, ret;

	kp = devm_kzalloc(dev, sizeof(*kp), GFP_KERNEL);
	if (!kp)
		return -ENOMEM;
	kp->dev = dev;

	kp->base = devm_platform_ioremap_resource(pdev, 0);
	if (IS_ERR(kp->base))
		return PTR_ERR(kp->base);

	irq = platform_get_irq(pdev, 0);
	if (irq < 0)
		return irq;

	kp->clk = devm_clk_get(dev, NULL);
	if (IS_ERR(kp->clk))
		return dev_err_probe(dev, PTR_ERR(kp->clk), "no clock\n");

	ret = matrix_keypad_parse_properties(dev, &kp->rows, &kp->cols);
	if (ret)
		return ret;
	if (kp->rows > KEYC_MAX_ROWS || kp->cols > KEYC_MAX_COLS)
		return dev_err_probe(dev, -EINVAL,
				     "at most %u rows and %u columns\n",
				     KEYC_MAX_ROWS, KEYC_MAX_COLS);
	kp->row_shift = get_count_order(kp->cols);

	/*
	 * The debounce register counts samples at the fixed 9 kHz sample
	 * rate.  Without the property, use the vendor value of 200.
	 */
	kp->debounce = KEYC_DEFAULT_DEBOUNCE;
	if (!device_property_read_u32(dev, "debounce-delay-ms", &debounce_ms))
		kp->debounce = clamp_t(u32, debounce_ms *
				       KEYC_DEFAULT_SAMPLE_HZ / 1000,
				       1, KEYC_DEBOUNCE_MASK);

	input = devm_input_allocate_device(dev);
	if (!input)
		return -ENOMEM;
	kp->input = input;

	input->name = "bcmring-keypad";
	input->phys = "keypad/input0";	/* same as the vendor driver */
	input->id.bustype = BUS_HOST;
	input->open = bcmring_keypad_open;
	input->close = bcmring_keypad_close;
	input_set_drvdata(input, kp);

	ret = matrix_keypad_build_keymap(NULL, NULL, kp->rows, kp->cols,
					 NULL, input);
	if (ret)
		return dev_err_probe(dev, ret, "failed to build keymap\n");

	input_set_capability(input, EV_MSC, MSC_SCAN);
	if (device_property_read_bool(dev, "autorepeat"))
		__set_bit(EV_REP, input->evbit);

	/* Make sure nothing fires before the device is opened. */
	bcmring_keypad_ctrl(kp, KEYC_CTRL_IRQ_ENABLE | KEYC_CTRL_ENABLE, 0);

	/*
	 * The vendor driver has no initial scan: its state starts as "all
	 * released" and changes only on interrupts.  Keep that, because the
	 * state registers' reset value is not known.
	 */
	ret = devm_request_irq(dev, irq, bcmring_keypad_irq, 0,
			       dev_name(dev), kp);
	if (ret)
		return dev_err_probe(dev, ret, "failed to request IRQ\n");

	platform_set_drvdata(pdev, kp);

	return input_register_device(input);
}

static int bcmring_keypad_suspend(struct device *dev)
{
	struct bcmring_keypad *kp = dev_get_drvdata(dev);

	/*
	 * TODO: the vendor suspend only shortens the debounce count to 15
	 * and queues up to eight events for replay on resume.  Wake-up
	 * behavior is untested, so this draft only mirrors the register
	 * write.
	 */
	guard(mutex)(&kp->input->mutex);
	if (input_device_enabled(kp->input))
		writel(KEYC_SUSPEND_DEBOUNCE, kp->base + KEYC_DEBOUNCE);

	return 0;
}

static int bcmring_keypad_resume(struct device *dev)
{
	struct bcmring_keypad *kp = dev_get_drvdata(dev);

	guard(mutex)(&kp->input->mutex);
	if (input_device_enabled(kp->input))
		writel(kp->debounce, kp->base + KEYC_DEBOUNCE);

	return 0;
}

static DEFINE_SIMPLE_DEV_PM_OPS(bcmring_keypad_pm_ops,
				bcmring_keypad_suspend, bcmring_keypad_resume);

static const struct of_device_id bcmring_keypad_of_match[] = {
	{ .compatible = "brcm,bcmring-keypad" },
	{ }
};
MODULE_DEVICE_TABLE(of, bcmring_keypad_of_match);

static struct platform_driver bcmring_keypad_driver = {
	.probe = bcmring_keypad_probe,
	.driver = {
		.name = "bcmring-keypad",
		.of_match_table = bcmring_keypad_of_match,
		.pm = pm_sleep_ptr(&bcmring_keypad_pm_ops),
	},
};
module_platform_driver(bcmring_keypad_driver);

MODULE_DESCRIPTION("Broadcom BCMRING keypad controller driver (first draft)");
MODULE_LICENSE("GPL");
