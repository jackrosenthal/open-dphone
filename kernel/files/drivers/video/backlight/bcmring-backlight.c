// SPDX-License-Identifier: GPL-2.0-only
/*
 * Backlight driver for the single-channel PWM block of Broadcom BCMRING
 * SoCs (BCM11107 family, including BCM11188/BCM11189), as wired on the
 * Digium/Sangoma D40, D45, D50, D70, D60, D62, and D65 phones.
 *
 * Written from the behavior recovered from the vendor 3.4.74 kernel (pwmHw_*
 * and lcd_backlight_* functions).
 *
 * The block is one 32-bit register:
 *
 *   bit  0     modulation enable
 *   bits 2:1   frequency select: 0..3 = about 48, 97, 195, 390 Hz
 *   bits 7:3   duty cycle, 0..31 (in 1/32 steps)
 *
 * The duty field cannot express "always on", so the vendor drivers switch
 * the pad to its GPIO function for the extremes.  The scale differs by board
 * family and is described in the devicetree:
 *
 *   D60/D62/D65 (vendor 3.4):   0..32; 1..31 are PWM duty steps, 0 and 32
 *                               drive the pad low/high as a GPIO.
 *                               (max-brightness = 32, brcm,full-on-level = 32;
 *                               these are the defaults.)
 *   D40/D45/D50/D70 (2.6.27):   0..31; 1..29 are PWM duty steps, 0 drives the
 *                               pad low and 30 and 31 both drive it high.
 *                               (max-brightness = 31, brcm,full-on-level = 30.)
 *
 * In both, levels from 1 to full-on-level - 1 program the duty cycle and
 * route the PWM output to the pad; level 0 and levels from full-on-level up
 * disable modulation, switch the same pad to its GPIO function, and drive
 * it low or high.  "default" pinctrl state = pad carries the PWM output,
 * "gpio" pinctrl state = pad is a GPIO, and the "gpios" property names that
 * GPIO.
 *
 * The register directly follows the LED matrix controller (0x80002000 to
 * 0x8000203f) and shares its bus-interface gate and clock; this driver maps
 * only its own 4 bytes and relies on the clock framework's reference counts
 * for the shared clocks.
 */

#include <linux/backlight.h>
#include <linux/bitfield.h>
#include <linux/bits.h>
#include <linux/clk.h>
#include <linux/err.h>
#include <linux/gpio/consumer.h>
#include <linux/io.h>
#include <linux/mod_devicetable.h>
#include <linux/module.h>
#include <linux/pinctrl/consumer.h>
#include <linux/platform_device.h>
#include <linux/property.h>

#define BCMRING_PWM_CTRL		0x00
#define BCMRING_PWM_CTRL_ENABLE		BIT(0)
#define BCMRING_PWM_CTRL_FREQ		GENMASK(2, 1)
#define BCMRING_PWM_CTRL_DUTY		GENMASK(7, 3)

#define BCMRING_BL_DUTY_STEPS		32	/* duty field counts 32nds */
#define BCMRING_BL_DEFAULT_FREQ_HZ	97

struct bcmring_bl {
	struct device *dev;
	void __iomem *base;
	struct gpio_desc *gpio;
	struct pinctrl *pinctrl;
	struct pinctrl_state *state_pwm;
	struct pinctrl_state *state_gpio;
	/* lowest level that means "pad driven high as a GPIO" */
	unsigned int full_on_level;
};

static void bcmring_bl_rmw(struct bcmring_bl *bl, u32 mask, u32 val)
{
	u32 ctrl = readl(bl->base + BCMRING_PWM_CTRL);

	ctrl = (ctrl & ~mask) | val;
	writel(ctrl, bl->base + BCMRING_PWM_CTRL);
}

/* Same thresholds as the vendor pwmHw_FreqSet(). */
static u32 bcmring_bl_freq_sel(u32 hz)
{
	if (hz > 389)
		return 3;
	if (hz > 194)
		return 2;
	if (hz > 96)
		return 1;
	return 0;
}

static bool bcmring_bl_can_use_gpio(struct bcmring_bl *bl)
{
	return bl->gpio && bl->state_gpio && bl->state_pwm;
}

/* Called with the backlight ops_lock held, which serializes register use. */
static int bcmring_bl_update_status(struct backlight_device *bd)
{
	struct bcmring_bl *bl = bl_get_data(bd);
	int level = backlight_get_brightness(bd);
	int ret;

	if (level > 0 && level < bl->full_on_level) {
		bcmring_bl_rmw(bl, BCMRING_PWM_CTRL_DUTY,
			       FIELD_PREP(BCMRING_PWM_CTRL_DUTY, level));
		bcmring_bl_rmw(bl, BCMRING_PWM_CTRL_ENABLE,
			       BCMRING_PWM_CTRL_ENABLE);
		if (bl->state_pwm) {
			ret = pinctrl_select_state(bl->pinctrl, bl->state_pwm);
			if (ret)
				return ret;
		}
		return 0;
	}

	if (bcmring_bl_can_use_gpio(bl)) {
		/* Vendor order: pad to GPIO, drive it, then stop the PWM. */
		ret = gpiod_direction_output(bl->gpio, level != 0);
		if (ret)
			return ret;
		ret = pinctrl_select_state(bl->pinctrl, bl->state_gpio);
		if (ret)
			return ret;
		bcmring_bl_rmw(bl, BCMRING_PWM_CTRL_ENABLE, 0);
		return 0;
	}

	/*
	 * Boards without the GPIO handover.  The pad level with modulation
	 * disabled, or with duty 0, is not known (the vendor driver never
	 * leaves the pad on the PWM function in either state), so full scale
	 * is approximated by duty 31/32 and off by duty 0 with modulation
	 * stopped.
	 */
	if (level) {
		bcmring_bl_rmw(bl, BCMRING_PWM_CTRL_DUTY,
			       FIELD_PREP(BCMRING_PWM_CTRL_DUTY,
					  BCMRING_BL_DUTY_STEPS - 1));
		bcmring_bl_rmw(bl, BCMRING_PWM_CTRL_ENABLE,
			       BCMRING_PWM_CTRL_ENABLE);
	} else {
		bcmring_bl_rmw(bl, BCMRING_PWM_CTRL_DUTY, 0);
		bcmring_bl_rmw(bl, BCMRING_PWM_CTRL_ENABLE, 0);
	}

	return 0;
}

static const struct backlight_ops bcmring_bl_ops = {
	.options = BL_CORE_SUSPENDRESUME,
	.update_status = bcmring_bl_update_status,
};

static int bcmring_bl_init_pins(struct bcmring_bl *bl)
{
	struct device *dev = bl->dev;

	bl->gpio = devm_gpiod_get_optional(dev, NULL, GPIOD_ASIS);
	if (IS_ERR(bl->gpio))
		return dev_err_probe(dev, PTR_ERR(bl->gpio),
				     "failed to get backlight GPIO\n");

	bl->pinctrl = devm_pinctrl_get(dev);
	if (IS_ERR(bl->pinctrl)) {
		if (PTR_ERR(bl->pinctrl) == -EPROBE_DEFER)
			return -EPROBE_DEFER;
		bl->pinctrl = NULL;
	} else {
		bl->state_pwm = pinctrl_lookup_state(bl->pinctrl,
						     PINCTRL_STATE_DEFAULT);
		bl->state_gpio = pinctrl_lookup_state(bl->pinctrl, "gpio");
		if (IS_ERR(bl->state_pwm))
			bl->state_pwm = NULL;
		if (IS_ERR(bl->state_gpio))
			bl->state_gpio = NULL;
	}

	if (!bcmring_bl_can_use_gpio(bl)) {
		dev_warn(dev,
			 "no GPIO handover (gpios + \"default\"/\"gpio\" pinctrl states); level 0 and levels from %u up are approximated\n",
			 bl->full_on_level);
		return 0;
	}

	/*
	 * The driver core has already selected "default" (PWM), but
	 * requesting the GPIO above switched the pad to its GPIO function
	 * behind the pin controller's back (pinctrl-single GPIO range).
	 * Select the "gpio" state so that the state the pinctrl core tracks
	 * matches the pad again; otherwise a first update to a PWM level
	 * would be skipped as "already in the default state".
	 */
	return pinctrl_select_state(bl->pinctrl, bl->state_gpio);
}

static int bcmring_bl_probe(struct platform_device *pdev)
{
	struct device *dev = &pdev->dev;
	struct backlight_properties props = { };
	struct backlight_device *bd;
	struct bcmring_bl *bl;
	struct clk *clk;
	u32 freq = BCMRING_BL_DEFAULT_FREQ_HZ;
	u32 max_brightness = BCMRING_BL_DUTY_STEPS;
	u32 brightness;
	int ret;

	bl = devm_kzalloc(dev, sizeof(*bl), GFP_KERNEL);
	if (!bl)
		return -ENOMEM;
	bl->dev = dev;

	bl->base = devm_platform_ioremap_resource(pdev, 0);
	if (IS_ERR(bl->base))
		return PTR_ERR(bl->base);

	/*
	 * "pwm" is the modulator clock (vendor clock "LED_PWM", 12.5 MHz
	 * from the crystal); "bus" is the register-interface gate the vendor
	 * code opens by hand (chipc BusIntfClock bit 16, shared with the LED
	 * matrix block).  Both are optional.
	 */
	clk = devm_clk_get_optional_enabled(dev, "bus");
	if (IS_ERR(clk))
		return dev_err_probe(dev, PTR_ERR(clk),
				     "failed to enable bus clock\n");

	clk = devm_clk_get_optional_enabled(dev, "pwm");
	if (IS_ERR(clk))
		return dev_err_probe(dev, PTR_ERR(clk),
				     "failed to enable PWM clock\n");

	/*
	 * Board-family scale, see the top of the file.  PWM levels are duty
	 * values, so the full-on threshold cannot exceed the number of duty
	 * steps, and there must be at least one PWM level below it.
	 */
	device_property_read_u32(dev, "max-brightness", &max_brightness);
	bl->full_on_level = max_brightness;
	device_property_read_u32(dev, "brcm,full-on-level", &bl->full_on_level);
	if (bl->full_on_level < 2 || bl->full_on_level > BCMRING_BL_DUTY_STEPS ||
	    max_brightness < bl->full_on_level)
		return dev_err_probe(dev, -EINVAL,
				     "invalid scale: max-brightness %u, full-on level %u\n",
				     max_brightness, bl->full_on_level);

	ret = bcmring_bl_init_pins(bl);
	if (ret)
		return ret;

	device_property_read_u32(dev, "brcm,pwm-frequency-hz", &freq);
	bcmring_bl_rmw(bl, BCMRING_PWM_CTRL_FREQ,
		       FIELD_PREP(BCMRING_PWM_CTRL_FREQ,
				  bcmring_bl_freq_sel(freq)));

	brightness = max_brightness;
	device_property_read_u32(dev, "default-brightness", &brightness);
	if (brightness > max_brightness)
		brightness = max_brightness;

	props.type = BACKLIGHT_RAW;
	props.scale = BACKLIGHT_SCALE_LINEAR;
	props.max_brightness = max_brightness;
	props.brightness = brightness;
	props.power = BACKLIGHT_POWER_ON;

	bd = devm_backlight_device_register(dev, dev_name(dev), dev, bl,
					    &bcmring_bl_ops, &props);
	if (IS_ERR(bd))
		return dev_err_probe(dev, PTR_ERR(bd),
				     "failed to register backlight\n");

	return backlight_update_status(bd);
}

static const struct of_device_id bcmring_bl_of_match[] = {
	{ .compatible = "brcm,bcmring-backlight" },
	{ }
};
MODULE_DEVICE_TABLE(of, bcmring_bl_of_match);

static struct platform_driver bcmring_bl_driver = {
	.probe = bcmring_bl_probe,
	.driver = {
		.name = "bcmring-backlight",
		.of_match_table = bcmring_bl_of_match,
	},
};
module_platform_driver(bcmring_bl_driver);

MODULE_DESCRIPTION("Broadcom BCMRING PWM backlight driver");
MODULE_LICENSE("GPL");
