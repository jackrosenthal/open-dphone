// SPDX-License-Identifier: GPL-2.0-only
/*
 * DRM driver for the SPI grayscale LCDs of the Digium/Sangoma D40, D45, D50,
 * and D70 desk phones.
 *
 * Written from the behavior recovered from the vendor 2.6.27.18 kernels
 * ("digium onephone fb", functions opfb_*).
 *
 * Two kinds of panel are handled:
 *
 * - The main panel (240x120 on D40/D45/D50, 320x160 on D70).  Four gray
 *   levels, three pixels per byte, MIPI-DCS style window commands, D/C line.
 *   The command set matches the Sitronix ST7586S as driven by
 *   drivers/gpu/drm/sitronix/st7586.c; the part number is not confirmed.
 *
 * - The D70 side display ("DSS", 113x357 as seen by the user).  Every
 *   command and command parameter is sent with D/C low; pixel data is 16 bits
 *   per group of three physical pixels (5-6-5).  The command set looks like
 *   an UltraChip UC16xx-family controller in a 64k-color mode driving a
 *   monochrome glass; the part number is not confirmed.
 *
 * On the D70 both panels share the SPI bus and the D/C line and differ in
 * chip select and reset line, so one driver instance owns both: the DT node
 * lists two chip selects and the driver creates an ancillary SPI device for
 * the second.  The result is one DRM device with one CRTC, plane, and
 * connector per panel.
 *
 * Like the vendor driver, every update re-sends the controller configuration
 * and then the whole frame, and an idle panel is refreshed the same way
 * every two seconds.
 */

#include <linux/backlight.h>
#include <linux/bits.h>
#include <linux/delay.h>
#include <linux/devm-helpers.h>
#include <linux/gpio/consumer.h>
#include <linux/lcd.h>
#include <linux/minmax.h>
#include <linux/mod_devicetable.h>
#include <linux/module.h>
#include <linux/mutex.h>
#include <linux/property.h>
#include <linux/sched.h>
#include <linux/spi/spi.h>
#include <linux/workqueue.h>

#include <video/mipi_display.h>

#include <drm/clients/drm_client_setup.h>
#include <drm/drm_atomic.h>
#include <drm/drm_atomic_helper.h>
#include <drm/drm_connector.h>
#include <drm/drm_damage_helper.h>
#include <drm/drm_drv.h>
#include <drm/drm_fbdev_shmem.h>
#include <drm/drm_format_helper.h>
#include <drm/drm_fourcc.h>
#include <drm/drm_framebuffer.h>
#include <drm/drm_gem_atomic_helper.h>
#include <drm/drm_gem_framebuffer_helper.h>
#include <drm/drm_gem_shmem_helper.h>
#include <drm/drm_managed.h>
#include <drm/drm_modes.h>
#include <drm/drm_probe_helper.h>
#include <drm/drm_rect.h>

#define DGL_MAX_PANELS		2
#define DGL_CMD_BUF_LEN		32
/*
 * Pixel data goes out in pieces of this size, about 1 ms on the wire at the
 * panels' 8.33 MHz, with a scheduling point after each: the SPI host sends a
 * piece without giving up the CPU (PL022 in polling mode), and the phone's
 * audio thread must not wait for a whole frame.
 */
#define DGL_DATA_CHUNK		1024

/*
 * Main panel commands.  Names follow drivers/gpu/drm/sitronix/st7586.c where
 * that driver uses the same opcode; the remaining opcodes are named by what
 * the vendor sequence suggests and are otherwise unexplained.
 */
#define DGL_MAIN_DISP_MODE_GRAY		0x38
#define DGL_MAIN_ENABLE_DDRAM		0x3a
#define DGL_MAIN_SET_DISP_DUTY		0xb0
#define DGL_MAIN_SET_FIRST_COM		0xb1	/* not in st7586.c */
#define DGL_MAIN_SET_NLINE_INV		0xb5
#define DGL_MAIN_SET_VOP		0xc0
#define DGL_MAIN_SET_BIAS_SYSTEM	0xc3
#define DGL_MAIN_SET_BOOST_LEVEL	0xc4
#define DGL_MAIN_ENABLE_ANALOG		0xd0
#define DGL_MAIN_AUTO_READ_CTRL		0xd7
#define DGL_MAIN_OTP_RW_CTRL		0xe0
#define DGL_MAIN_OTP_CTRL_OUT		0xe1
#define DGL_MAIN_OTP_READ		0xe3
#define DGL_MAIN_UNKNOWN_EB		0xeb	/* not in st7586.c */
#define DGL_MAIN_FRAME_RATE_F0		0xf0	/* not in st7586.c */
#define DGL_MAIN_FRAME_RATE_F1		0xf1	/* not in st7586.c */

/* DSS commands that this driver has to patch or generate itself. */
#define DGL_DSS_SET_VBIAS_POT		0x81	/* followed by the value */
#define DGL_DSS_DISPLAY_OFF		0xa8	/* vendor only ever sends 0xaf */
#define DGL_DSS_NOP			0xe3
#define DGL_DSS_SYSTEM_RESET		0xe2

enum dgl_ctrl {
	DGL_CTRL_MAIN,
	DGL_CTRL_DSS,
};

struct dgl_reg {
	u8 cmd;
	u8 len;
	u8 data[4];
};

struct dgl_panel_desc {
	const char *name;
	enum dgl_ctrl ctrl;
	struct drm_display_mode mode;
	/* Main panels: configuration sent after the SET_VOP command. */
	const struct dgl_reg *regs;
	unsigned int num_regs;
	u16 max_contrast;
	u16 default_contrast;
};

struct dgl_device_desc {
	unsigned int num_panels;
	const struct dgl_panel_desc *panels[DGL_MAX_PANELS];
};

struct dgl_device;

struct dgl_panel {
	struct dgl_device *dgl;
	const struct dgl_panel_desc *desc;
	struct spi_device *spi;
	struct gpio_desc *reset;
	struct backlight_device *backlight;
	struct lcd_device *lcd;

	struct drm_plane plane;
	struct drm_crtc crtc;
	struct drm_encoder encoder;
	struct drm_connector connector;

	struct delayed_work refresh;

	/* Everything below is protected by dgl_device.lock. */
	u8 *cmd;		/* DMA-safe scratch, DGL_CMD_BUF_LEN bytes */
	u8 *gray;		/* 8-bit copy of the frame, width * height */
	u8 *shadow;		/* frame in panel format */
	size_t shadow_len;
	unsigned int stride;	/* main: bytes per row; DSS: bytes per column */
	unsigned int contrast;
	bool enabled;
};

struct dgl_device {
	struct drm_device drm;
	struct gpio_desc *dc;
	/* Serializes the shared D/C line and all per-panel buffers. */
	struct mutex lock;
	unsigned int num_panels;
	struct dgl_panel panels[DGL_MAX_PANELS];

	/* D70 side-display backlight: one enable and five level lines. */
	struct gpio_desc *dss_bl_enable;
	struct gpio_descs *dss_bl_level;
};

static unsigned int refresh_ms = 2000;
module_param(refresh_ms, uint, 0644);
MODULE_PARM_DESC(refresh_ms,
		 "Interval of the idle re-initialization and full refresh in ms (0 disables; vendor: 2000)");

/* ------------------------------------------------------------------------
 * SPI transport
 */

static int dgl_write(struct dgl_panel *p, bool data, const void *buf, size_t len)
{
	gpiod_set_value_cansleep(p->dgl->dc, data);

	return spi_write(p->spi, buf, len);
}

/*
 * Pixel data, in pieces.  The chip select stays asserted from the first
 * piece to the last, so the controller sees one write.
 */
static int dgl_write_pixels(struct dgl_panel *p, const u8 *buf, size_t len)
{
	int err;

	gpiod_set_value_cansleep(p->dgl->dc, 1);

	while (len) {
		size_t n = min_t(size_t, len, DGL_DATA_CHUNK);
		struct spi_transfer xfer = {
			.tx_buf = buf,
			.len = n,
			/* On the last transfer of a message: keep it selected. */
			.cs_change = len > n,
		};

		err = spi_sync_transfer(p->spi, &xfer, 1);
		if (err)
			return err;

		buf += n;
		len -= n;
		cond_resched();
	}

	return 0;
}

/* ------------------------------------------------------------------------
 * Main panel
 */

static void dgl_main_cmd_buf(struct dgl_panel *p, int *err, u8 cmd,
			     const u8 *par, size_t len)
{
	if (*err)
		return;

	p->cmd[0] = cmd;
	*err = dgl_write(p, false, p->cmd, 1);
	if (*err || !len)
		return;

	memcpy(p->cmd, par, len);
	*err = dgl_write(p, true, p->cmd, len);
}

#define dgl_main_cmd(p, err, cmd, seq...) \
({ \
	static const u8 d[] = { seq }; \
	dgl_main_cmd_buf(p, err, cmd, d, ARRAY_SIZE(d)); \
})

/* D40, D45, and D50: opfb_reinit_lcd_d40(). */
static const struct dgl_reg dgl_d40_main_regs[] = {
	{ DGL_MAIN_SET_BIAS_SYSTEM,	1, { 0x02 } },
	{ DGL_MAIN_SET_BOOST_LEVEL,	1, { 0x05 } },
	{ DGL_MAIN_ENABLE_ANALOG,	1, { 0x1d } },
	{ DGL_MAIN_SET_NLINE_INV,	1, { 0x8c } },
	{ DGL_MAIN_DISP_MODE_GRAY,	0 },
	{ DGL_MAIN_ENABLE_DDRAM,	1, { 0x02 } },
	{ MIPI_DCS_SET_ADDRESS_MODE,	1, { 0xc8 } },
	{ DGL_MAIN_UNKNOWN_EB,		2, { 0x1e, 0x02 } },
	{ DGL_MAIN_SET_DISP_DUTY,	1, { 0x77 } },
	{ MIPI_DCS_SET_SCROLL_START,	1, { 0x28 } },
	{ DGL_MAIN_SET_FIRST_COM,	1, { 0x28 } },
	{ DGL_MAIN_FRAME_RATE_F1,	4, { 0x12, 0x12, 0x12, 0x12 } },
	{ MIPI_DCS_ENTER_INVERT_MODE,	0 },
};

/* D70: opfb_reinit_lcd_d70(). */
static const struct dgl_reg dgl_d70_main_regs[] = {
	{ DGL_MAIN_SET_BIAS_SYSTEM,	1, { 0x01 } },
	{ DGL_MAIN_SET_BOOST_LEVEL,	1, { 0x06 } },
	{ DGL_MAIN_ENABLE_ANALOG,	1, { 0x1d } },
	{ DGL_MAIN_SET_NLINE_INV,	1, { 0x8a } },
	{ DGL_MAIN_DISP_MODE_GRAY,	0 },
	{ DGL_MAIN_ENABLE_DDRAM,	1, { 0x02 } },
	{ MIPI_DCS_SET_ADDRESS_MODE,	1, { 0xc8 } },
	{ DGL_MAIN_UNKNOWN_EB,		2, { 0x17, 0x03 } },
	{ DGL_MAIN_SET_DISP_DUTY,	1, { 0x9f } },
	{ MIPI_DCS_SET_SCROLL_START,	1, { 0x00 } },
	{ DGL_MAIN_SET_FIRST_COM,	1, { 0x00 } },
	{ DGL_MAIN_FRAME_RATE_F0,	4, { 0x06, 0x0b, 0x0d, 0x15 } },
	{ MIPI_DCS_ENTER_INVERT_MODE,	0 },
	{ DGL_MAIN_UNKNOWN_EB,		2, { 0x1d, 0x02 } },
};

static void dgl_main_set_vop(struct dgl_panel *p, int *err)
{
	/* Nine-bit value: low byte first, then bit 8. */
	const u8 vop[2] = { p->contrast & 0xff, (p->contrast >> 8) & 0x01 };

	dgl_main_cmd_buf(p, err, DGL_MAIN_SET_VOP, vop, sizeof(vop));
}

/*
 * Controller configuration.  The vendor driver runs the "cold" variant once
 * after the hardware reset and the "warm" variant in front of every frame.
 */
static int dgl_main_configure(struct dgl_panel *p, bool cold)
{
	const struct dgl_panel_desc *desc = p->desc;
	unsigned int i;
	int err = 0;

	dgl_main_cmd(p, &err, DGL_MAIN_AUTO_READ_CTRL, 0x9f);
	dgl_main_cmd(p, &err, DGL_MAIN_OTP_RW_CTRL, 0x00);
	if (err)
		return err;
	usleep_range(1000, 2000);

	if (cold) {
		dgl_main_cmd(p, &err, DGL_MAIN_OTP_READ);
		msleep(20);
	}

	dgl_main_cmd(p, &err, DGL_MAIN_OTP_CTRL_OUT);
	dgl_main_cmd(p, &err, MIPI_DCS_EXIT_SLEEP_MODE);

	if (cold) {
		dgl_main_cmd(p, &err, MIPI_DCS_SET_DISPLAY_OFF);
		msleep(50);
	}

	dgl_main_set_vop(p, &err);

	for (i = 0; i < desc->num_regs; i++)
		dgl_main_cmd_buf(p, &err, desc->regs[i].cmd,
				 desc->regs[i].data, desc->regs[i].len);

	return err;
}

static int dgl_main_flush(struct dgl_panel *p)
{
	const struct drm_display_mode *mode = &p->desc->mode;
	/* Columns count bytes, that is, groups of three pixels. */
	const u8 cols[4] = { 0, 0, 0, p->stride - 1 };
	const u8 rows[4] = { 0, 0, 0, mode->vdisplay - 1 };
	int err;

	err = dgl_main_configure(p, false);

	dgl_main_cmd_buf(p, &err, MIPI_DCS_SET_COLUMN_ADDRESS, cols, sizeof(cols));
	dgl_main_cmd_buf(p, &err, MIPI_DCS_SET_PAGE_ADDRESS, rows, sizeof(rows));
	dgl_main_cmd(p, &err, MIPI_DCS_WRITE_MEMORY_START);
	if (err)
		return err;

	return dgl_write_pixels(p, p->shadow, p->shadow_len);
}

/*
 * Three pixels per byte.  With display inversion on, as the vendor
 * configures it, the two-bit level L (0 = black, 3 = white) is stored as
 * 000, 010, 100, or 111 in bits 7:5 and 4:2, and as L itself in bits 1:0.
 */
static const u8 dgl_main_lut[4] = { 0x0, 0x2, 0x4, 0x7 };

static void dgl_main_pack(struct dgl_panel *p, const struct drm_rect *clip)
{
	const unsigned int width = p->desc->mode.hdisplay;
	unsigned int first = clip->x1 / 3;
	unsigned int last = (clip->x2 - 1) / 3;
	unsigned int x, y, i;

	for (y = clip->y1; y < clip->y2; y++) {
		const u8 *src = p->gray + y * width;
		u8 *dst = p->shadow + y * p->stride;

		for (x = first; x <= last; x++) {
			u8 lvl[3];

			/* Columns past the visible width are sent as white. */
			for (i = 0; i < 3; i++)
				lvl[i] = x * 3 + i < width ?
					 src[x * 3 + i] >> 6 : 3;

			dst[x] = dgl_main_lut[lvl[0]] << 5 |
				 dgl_main_lut[lvl[1]] << 2 |
				 dgl_main_lut[lvl[2]] >> 1;
		}
	}
}

static int dgl_main_power_on(struct dgl_panel *p)
{
	int err;

	/* Vendor: 1 us low, then 120 ms. */
	gpiod_set_value_cansleep(p->reset, 1);
	usleep_range(20, 100);
	gpiod_set_value_cansleep(p->reset, 0);
	msleep(120);

	err = dgl_main_configure(p, true);
	if (err)
		return err;

	/*
	 * The vendor switches the display on first and fills the memory
	 * afterwards; fill it first to avoid showing its power-on content.
	 */
	err = dgl_main_flush(p);
	dgl_main_cmd(p, &err, MIPI_DCS_SET_DISPLAY_ON);

	return err;
}

static void dgl_main_power_off(struct dgl_panel *p)
{
	int err = 0;

	/* The vendor driver never switches the panel off. */
	dgl_main_cmd(p, &err, MIPI_DCS_SET_DISPLAY_OFF);
}

/* ------------------------------------------------------------------------
 * D70 side display
 */

#define DGL_DSS_REGS_RESET_IDX	0
#define DGL_DSS_REGS_POT_IDX	7

/*
 * opfb_reinit_lcd_d70_dss(): one 26-byte burst with D/C low.  Byte 0 is a
 * NOP that becomes a system reset on the first call; byte 7 is replaced
 * with the contrast value.
 */
static const u8 dgl_dss_regs[] = {
	DGL_DSS_NOP, 0x2b, 0x24, 0xc0, 0xa2, 0xe9,
	DGL_DSS_SET_VBIAS_POT, 0x70,
	0xc8, 0x18, 0x00, 0xd1, 0xd6, 0x8b, 0xf8, 0x84,
	0x31, 0x1d, 0xa7, 0xaf, 0x30, 0x50, 0xf1, 0x71,
	0x31, 0x2d,
};

/*
 * opfb_handle_damage_d70_dss(): window columns 0..118 and rows 0..112, then
 * the write position (column 0, row 0).
 */
static const u8 dgl_dss_window[] = {
	0xf4, 0x00, 0xf6, 0x76, 0xf5, 0x00, 0xf7, 0x70,
	0x00, 0x10, 0x60, 0x70,
};

static int dgl_dss_configure(struct dgl_panel *p, bool cold)
{
	BUILD_BUG_ON(sizeof(dgl_dss_regs) > DGL_CMD_BUF_LEN);

	memcpy(p->cmd, dgl_dss_regs, sizeof(dgl_dss_regs));
	if (cold)
		p->cmd[DGL_DSS_REGS_RESET_IDX] = DGL_DSS_SYSTEM_RESET;
	p->cmd[DGL_DSS_REGS_POT_IDX] = p->contrast;

	return dgl_write(p, false, p->cmd, sizeof(dgl_dss_regs));
}

static int dgl_dss_flush(struct dgl_panel *p)
{
	int err;

	err = dgl_dss_configure(p, false);
	if (err)
		return err;

	memcpy(p->cmd, dgl_dss_window, sizeof(dgl_dss_window));
	err = dgl_write(p, false, p->cmd, sizeof(dgl_dss_window));
	if (err)
		return err;

	return dgl_write_pixels(p, p->shadow, p->shadow_len);
}

/*
 * The glass is mounted rotated: a framebuffer column is a controller row
 * and three framebuffer rows form one controller column, whose 16-bit word
 * carries them as 5, 6, and 5 bits.  The data stream is column-major.
 */
static void dgl_dss_pack(struct dgl_panel *p, const struct drm_rect *clip)
{
	const unsigned int width = p->desc->mode.hdisplay;
	unsigned int first = clip->y1 / 3;
	unsigned int last = (clip->y2 - 1) / 3;
	unsigned int x, col;

	for (col = first; col <= last; col++) {
		const u8 *src = p->gray + col * 3 * width;
		u8 *dst = p->shadow + col * p->stride;

		for (x = clip->x1; x < clip->x2; x++) {
			u8 a = src[x];
			u8 b = src[x + width];
			u8 c = src[x + 2 * width];

			dst[2 * x] = (a & 0xf8) | (b >> 5);
			dst[2 * x + 1] = ((b << 3) & 0xe0) | (c >> 3);
		}
	}
}

static int dgl_dss_power_on(struct dgl_panel *p)
{
	int err;

	gpiod_set_value_cansleep(p->reset, 1);
	usleep_range(2000, 3000);
	gpiod_set_value_cansleep(p->reset, 0);
	msleep(300);

	/*
	 * The vendor driver writes 40960 zero bytes of pixel data here,
	 * before any configuration.  The first flush below overwrites the
	 * whole window instead.
	 */
	err = dgl_dss_configure(p, true);
	if (err)
		return err;
	/* Not in the vendor sequence: let the software reset settle. */
	usleep_range(5000, 6000);

	return dgl_dss_flush(p);
}

static void dgl_dss_power_off(struct dgl_panel *p)
{
	/*
	 * The vendor driver never switches the panel off; 0xa8 is its
	 * display-enable command (0xaf) with the enable bits cleared, which
	 * is an inference from the command encoding.
	 */
	p->cmd[0] = DGL_DSS_DISPLAY_OFF;
	dgl_write(p, false, p->cmd, 1);
}

/* ------------------------------------------------------------------------
 * Common panel handling
 */

static int dgl_panel_flush(struct dgl_panel *p)
{
	if (p->desc->ctrl == DGL_CTRL_DSS)
		return dgl_dss_flush(p);

	return dgl_main_flush(p);
}

static void dgl_panel_pack(struct dgl_panel *p, const struct drm_rect *clip)
{
	if (p->desc->ctrl == DGL_CTRL_DSS)
		dgl_dss_pack(p, clip);
	else
		dgl_main_pack(p, clip);
}

/*
 * (Re)starts the idle interval: called after every frame, so that, like the
 * vendor driver, only a panel that has not been sent anything for the
 * interval is refreshed.
 */
static void dgl_panel_schedule_refresh(struct dgl_panel *p)
{
	if (refresh_ms)
		mod_delayed_work(system_dfl_wq, &p->refresh,
				 msecs_to_jiffies(refresh_ms));
}

/*
 * The vendor driver re-initializes the controller and repaints a panel
 * that has not been sent for two seconds even without new content,
 * presumably so that a controller upset by ESD recovers by itself.
 */
static void dgl_panel_refresh_work(struct work_struct *work)
{
	struct dgl_panel *p = container_of(to_delayed_work(work),
					   struct dgl_panel, refresh);
	struct dgl_device *dgl = p->dgl;
	bool enabled;
	int idx;

	if (!drm_dev_enter(&dgl->drm, &idx))
		return;

	mutex_lock(&dgl->lock);
	enabled = p->enabled;
	if (enabled)
		dgl_panel_flush(p);
	mutex_unlock(&dgl->lock);

	drm_dev_exit(idx);

	if (enabled)
		dgl_panel_schedule_refresh(p);
}

/* ------------------------------------------------------------------------
 * Contrast (LCD class device)
 */

static int dgl_lcd_get_contrast(struct lcd_device *lcd)
{
	struct dgl_panel *p = lcd_get_data(lcd);

	return p->contrast;
}

static int dgl_lcd_set_contrast(struct lcd_device *lcd, int contrast)
{
	struct dgl_panel *p = lcd_get_data(lcd);
	int err = 0;

	if (contrast < 0 || contrast > p->desc->max_contrast)
		return -EINVAL;

	mutex_lock(&p->dgl->lock);
	p->contrast = contrast;
	if (p->enabled) {
		if (p->desc->ctrl == DGL_CTRL_DSS) {
			p->cmd[0] = DGL_DSS_SET_VBIAS_POT;
			p->cmd[1] = contrast;
			err = dgl_write(p, false, p->cmd, 2);
		} else {
			dgl_main_set_vop(p, &err);
		}
	}
	mutex_unlock(&p->dgl->lock);

	return err;
}

static const struct lcd_ops dgl_lcd_ops = {
	.get_contrast = dgl_lcd_get_contrast,
	.set_contrast = dgl_lcd_set_contrast,
};

/* ------------------------------------------------------------------------
 * D70 side-display backlight
 */

static int dgl_dss_bl_update_status(struct backlight_device *bd)
{
	struct dgl_device *dgl = bl_get_data(bd);
	int level = backlight_get_brightness(bd);
	unsigned int i;

	/* Binary-weighted level lines, least significant first. */
	for (i = 0; i < dgl->dss_bl_level->ndescs; i++)
		gpiod_set_value_cansleep(dgl->dss_bl_level->desc[i],
					 !!(level & BIT(i)));
	gpiod_set_value_cansleep(dgl->dss_bl_enable, level != 0);

	return 0;
}

static const struct backlight_ops dgl_dss_bl_ops = {
	.update_status = dgl_dss_bl_update_status,
};

static int dgl_dss_bl_init(struct dgl_device *dgl, struct dgl_panel *p)
{
	struct device *dev = dgl->drm.dev;
	struct backlight_properties props = {
		.type = BACKLIGHT_RAW,
		.power = BACKLIGHT_POWER_OFF,
	};

	dgl->dss_bl_enable = devm_gpiod_get_optional(dev, "dss-backlight-enable",
						     GPIOD_OUT_LOW);
	if (IS_ERR(dgl->dss_bl_enable))
		return dev_err_probe(dev, PTR_ERR(dgl->dss_bl_enable),
				     "Failed to get DSS backlight enable GPIO\n");
	if (!dgl->dss_bl_enable)
		return 0;

	dgl->dss_bl_level = devm_gpiod_get_array(dev, "dss-backlight-level",
						 GPIOD_OUT_LOW);
	if (IS_ERR(dgl->dss_bl_level))
		return dev_err_probe(dev, PTR_ERR(dgl->dss_bl_level),
				     "Failed to get DSS backlight level GPIOs\n");

	props.max_brightness = BIT(dgl->dss_bl_level->ndescs) - 1;
	/*
	 * The vendor driver leaves this backlight off until user space sets
	 * a level; its user-visible scale only reaches 16 of 31.
	 */
	props.brightness = 0;

	p->backlight = devm_backlight_device_register(dev, "digium-dss", dev,
						      dgl, &dgl_dss_bl_ops,
						      &props);

	return PTR_ERR_OR_ZERO(p->backlight);
}

/* ------------------------------------------------------------------------
 * DRM plane
 */

static const u32 dgl_formats[] = {
	DRM_FORMAT_XRGB8888,
	DRM_FORMAT_R8,
};

static int dgl_plane_atomic_check(struct drm_plane *plane,
				  struct drm_atomic_commit *state)
{
	struct drm_plane_state *plane_state = drm_atomic_get_new_plane_state(state, plane);
	struct drm_crtc_state *crtc_state = NULL;

	if (plane_state->crtc)
		crtc_state = drm_atomic_get_new_crtc_state(state, plane_state->crtc);

	return drm_atomic_helper_check_plane_state(plane_state, crtc_state,
						   DRM_PLANE_NO_SCALING,
						   DRM_PLANE_NO_SCALING,
						   false, false);
}

static void dgl_plane_atomic_update(struct drm_plane *plane,
				    struct drm_atomic_commit *state)
{
	struct dgl_panel *p = container_of(plane, struct dgl_panel, plane);
	struct dgl_device *dgl = p->dgl;
	struct drm_plane_state *plane_state = drm_atomic_get_new_plane_state(state, plane);
	struct drm_plane_state *old_state = drm_atomic_get_old_plane_state(state, plane);
	struct drm_shadow_plane_state *shadow = to_drm_shadow_plane_state(plane_state);
	struct drm_framebuffer *fb = plane_state->fb;
	const unsigned int width = p->desc->mode.hdisplay;
	unsigned int pitch = width;
	struct drm_rect src_clip, clip;
	struct iosys_map dst;
	int idx, err;

	if (!fb || !plane_state->crtc || !plane_state->crtc->state->active)
		return;

	if (!drm_atomic_helper_damage_merged(old_state, plane_state, &src_clip))
		return;

	/* Damage is in framebuffer coordinates; the panel copy is not. */
	clip = src_clip;
	drm_rect_translate(&clip, -(plane_state->src.x1 >> 16),
			   -(plane_state->src.y1 >> 16));

	if (!drm_dev_enter(&dgl->drm, &idx))
		return;

	mutex_lock(&dgl->lock);

	iosys_map_set_vaddr(&dst, p->gray + clip.y1 * width + clip.x1);
	if (fb->format->format == DRM_FORMAT_R8)
		drm_fb_memcpy(&dst, &pitch, shadow->data, fb, &src_clip);
	else
		drm_fb_xrgb8888_to_gray8(&dst, &pitch, shadow->data, fb,
					 &src_clip, &shadow->fmtcnv_state);

	dgl_panel_pack(p, &clip);

	err = p->enabled ? dgl_panel_flush(p) : 0;
	if (p->enabled)
		dgl_panel_schedule_refresh(p);

	mutex_unlock(&dgl->lock);

	if (err)
		dev_err_once(dgl->drm.dev, "%s: update failed: %d\n",
			     p->desc->name, err);

	drm_dev_exit(idx);
}

static const struct drm_plane_helper_funcs dgl_plane_helper_funcs = {
	DRM_GEM_SHADOW_PLANE_HELPER_FUNCS,
	.atomic_check = dgl_plane_atomic_check,
	.atomic_update = dgl_plane_atomic_update,
};

static const struct drm_plane_funcs dgl_plane_funcs = {
	.update_plane = drm_atomic_helper_update_plane,
	.disable_plane = drm_atomic_helper_disable_plane,
	.destroy = drm_plane_cleanup,
	DRM_GEM_SHADOW_PLANE_FUNCS,
};

/* ------------------------------------------------------------------------
 * DRM CRTC
 */

static enum drm_mode_status dgl_crtc_mode_valid(struct drm_crtc *crtc,
						const struct drm_display_mode *mode)
{
	struct dgl_panel *p = container_of(crtc, struct dgl_panel, crtc);

	return drm_crtc_helper_mode_valid_fixed(crtc, mode, &p->desc->mode);
}

static int dgl_crtc_atomic_check(struct drm_crtc *crtc,
				 struct drm_atomic_commit *state)
{
	struct drm_crtc_state *crtc_state = drm_atomic_get_new_crtc_state(state, crtc);
	int ret;

	if (crtc_state->enable) {
		ret = drm_atomic_helper_check_crtc_primary_plane(crtc_state);
		if (ret)
			return ret;
	}

	return drm_atomic_add_affected_planes(state, crtc);
}

static void dgl_crtc_atomic_enable(struct drm_crtc *crtc,
				   struct drm_atomic_commit *state)
{
	struct dgl_panel *p = container_of(crtc, struct dgl_panel, crtc);
	struct dgl_device *dgl = p->dgl;
	int idx, err;

	if (!drm_dev_enter(&dgl->drm, &idx))
		return;

	mutex_lock(&dgl->lock);
	if (p->desc->ctrl == DGL_CTRL_DSS)
		err = dgl_dss_power_on(p);
	else
		err = dgl_main_power_on(p);
	p->enabled = !err;
	mutex_unlock(&dgl->lock);

	if (err) {
		dev_err(dgl->drm.dev, "%s: power-on failed: %d\n",
			p->desc->name, err);
	} else {
		backlight_enable(p->backlight);
		dgl_panel_schedule_refresh(p);
	}

	drm_dev_exit(idx);
}

static void dgl_crtc_atomic_disable(struct drm_crtc *crtc,
				    struct drm_atomic_commit *state)
{
	struct dgl_panel *p = container_of(crtc, struct dgl_panel, crtc);
	struct dgl_device *dgl = p->dgl;

	/*
	 * Not guarded by drm_dev_enter(): the panel should also be switched
	 * off when the driver is unbound.
	 */
	backlight_disable(p->backlight);

	mutex_lock(&dgl->lock);
	p->enabled = false;
	mutex_unlock(&dgl->lock);

	cancel_delayed_work_sync(&p->refresh);

	mutex_lock(&dgl->lock);
	if (p->desc->ctrl == DGL_CTRL_DSS)
		dgl_dss_power_off(p);
	else
		dgl_main_power_off(p);
	mutex_unlock(&dgl->lock);
}

static const struct drm_crtc_helper_funcs dgl_crtc_helper_funcs = {
	.mode_valid = dgl_crtc_mode_valid,
	.atomic_check = dgl_crtc_atomic_check,
	.atomic_enable = dgl_crtc_atomic_enable,
	.atomic_disable = dgl_crtc_atomic_disable,
};

static const struct drm_crtc_funcs dgl_crtc_funcs = {
	.reset = drm_atomic_helper_crtc_reset,
	.destroy = drm_crtc_cleanup,
	.set_config = drm_atomic_helper_set_config,
	.page_flip = drm_atomic_helper_page_flip,
	.atomic_duplicate_state = drm_atomic_helper_crtc_duplicate_state,
	.atomic_destroy_state = drm_atomic_helper_crtc_destroy_state,
};

/* ------------------------------------------------------------------------
 * DRM encoder, connector, and mode configuration
 */

static const struct drm_encoder_funcs dgl_encoder_funcs = {
	.destroy = drm_encoder_cleanup,
};

static int dgl_connector_get_modes(struct drm_connector *connector)
{
	struct dgl_panel *p = container_of(connector, struct dgl_panel, connector);

	return drm_connector_helper_get_modes_fixed(connector, &p->desc->mode);
}

static const struct drm_connector_helper_funcs dgl_connector_helper_funcs = {
	.get_modes = dgl_connector_get_modes,
};

static const struct drm_connector_funcs dgl_connector_funcs = {
	.reset = drm_atomic_helper_connector_reset,
	.fill_modes = drm_helper_probe_single_connector_modes,
	.destroy = drm_connector_cleanup,
	.atomic_duplicate_state = drm_atomic_helper_connector_duplicate_state,
	.atomic_destroy_state = drm_atomic_helper_connector_destroy_state,
};

static const struct drm_mode_config_funcs dgl_mode_config_funcs = {
	.fb_create = drm_gem_fb_create_with_dirty,
	.atomic_check = drm_atomic_helper_check,
	.atomic_commit = drm_atomic_helper_commit,
};

static int dgl_pipe_init(struct dgl_device *dgl, struct dgl_panel *p)
{
	struct drm_device *drm = &dgl->drm;
	int ret;

	ret = drm_universal_plane_init(drm, &p->plane, 0, &dgl_plane_funcs,
				       dgl_formats, ARRAY_SIZE(dgl_formats),
				       NULL, DRM_PLANE_TYPE_PRIMARY, NULL);
	if (ret)
		return ret;
	drm_plane_helper_add(&p->plane, &dgl_plane_helper_funcs);
	drm_plane_enable_fb_damage_clips(&p->plane);

	ret = drm_crtc_init_with_planes(drm, &p->crtc, &p->plane, NULL,
					&dgl_crtc_funcs, NULL);
	if (ret)
		return ret;
	drm_crtc_helper_add(&p->crtc, &dgl_crtc_helper_funcs);

	ret = drm_encoder_init(drm, &p->encoder, &dgl_encoder_funcs,
			       DRM_MODE_ENCODER_NONE, NULL);
	if (ret)
		return ret;
	p->encoder.possible_crtcs = drm_crtc_mask(&p->crtc);

	ret = drm_connector_init(drm, &p->connector, &dgl_connector_funcs,
				 DRM_MODE_CONNECTOR_SPI);
	if (ret)
		return ret;
	drm_connector_helper_add(&p->connector, &dgl_connector_helper_funcs);

	return drm_connector_attach_encoder(&p->connector, &p->encoder);
}

DEFINE_DRM_GEM_FOPS(dgl_fops);

static const struct drm_driver dgl_drm_driver = {
	.driver_features	= DRIVER_GEM | DRIVER_MODESET | DRIVER_ATOMIC,
	.fops			= &dgl_fops,
	DRM_GEM_SHMEM_DRIVER_OPS,
	DRM_FBDEV_SHMEM_DRIVER_OPS,
	.name			= "digium-gray-lcd",
	.desc			= "Digium D40/D45/D50/D70 grayscale LCD",
	.major			= 1,
	.minor			= 0,
};

/* ------------------------------------------------------------------------
 * Panel descriptions
 */

/*
 * Physical sizes are derived from the advertised diagonals (3.5 and 4.5
 * inch, both 2:1); the size of the side display is not known.
 */
static const struct dgl_panel_desc dgl_d40_main = {
	.name = "d40-main",
	.ctrl = DGL_CTRL_MAIN,
	.mode = { DRM_SIMPLE_MODE(240, 120, 80, 40) },
	.regs = dgl_d40_main_regs,
	.num_regs = ARRAY_SIZE(dgl_d40_main_regs),
	.max_contrast = 511,
	.default_contrast = 260,
};

static const struct dgl_panel_desc dgl_d70_main = {
	.name = "d70-main",
	.ctrl = DGL_CTRL_MAIN,
	.mode = { DRM_SIMPLE_MODE(320, 160, 102, 51) },
	.regs = dgl_d70_main_regs,
	.num_regs = ARRAY_SIZE(dgl_d70_main_regs),
	.max_contrast = 511,
	.default_contrast = 344,
};

static const struct dgl_panel_desc dgl_d70_dss = {
	.name = "d70-dss",
	.ctrl = DGL_CTRL_DSS,
	.mode = { DRM_SIMPLE_MODE(113, 357, 0, 0) },
	.max_contrast = 255,
	.default_contrast = 130,
};

static const struct dgl_device_desc dgl_d40_desc = {
	.num_panels = 1,
	.panels = { &dgl_d40_main },
};

static const struct dgl_device_desc dgl_d70_desc = {
	.num_panels = 2,
	.panels = { &dgl_d70_main, &dgl_d70_dss },
};

/* ------------------------------------------------------------------------
 * Probe
 */

static int dgl_panel_init(struct dgl_device *dgl, unsigned int index,
			  const struct dgl_panel_desc *desc,
			  struct spi_device *spi)
{
	struct device *dev = dgl->drm.dev;
	struct dgl_panel *p = &dgl->panels[index];
	const unsigned int width = desc->mode.hdisplay;
	const unsigned int height = desc->mode.vdisplay;
	struct drm_rect full = DRM_RECT_INIT(0, 0, width, height);
	int ret;

	p->dgl = dgl;
	p->desc = desc;
	p->spi = spi;
	p->contrast = desc->default_contrast;

	ret = devm_delayed_work_autocancel(dev, &p->refresh,
					   dgl_panel_refresh_work);
	if (ret)
		return ret;

	/* The reset lines idle high; describe them as active low. */
	p->reset = devm_gpiod_get_index(dev, "reset", index, GPIOD_OUT_LOW);
	if (IS_ERR(p->reset))
		return dev_err_probe(dev, PTR_ERR(p->reset),
				     "%s: failed to get reset GPIO\n", desc->name);

	if (desc->ctrl == DGL_CTRL_DSS) {
		/* 3 rows -> one 16-bit word; one column of words per stride. */
		p->stride = width * 2;
		p->shadow_len = p->stride * (height / 3);
	} else {
		p->stride = DIV_ROUND_UP(width, 3);
		p->shadow_len = p->stride * height;
	}

	p->cmd = devm_kzalloc(dev, DGL_CMD_BUF_LEN, GFP_KERNEL);
	p->gray = devm_kmalloc(dev, width * height, GFP_KERNEL);
	p->shadow = devm_kzalloc(dev, p->shadow_len, GFP_KERNEL);
	if (!p->cmd || !p->gray || !p->shadow)
		return -ENOMEM;

	/* Start out white, like the vendor framebuffer. */
	memset(p->gray, 0xff, width * height);
	dgl_panel_pack(p, &full);

	p->lcd = devm_lcd_device_register(dev, desc->name, dev, p, &dgl_lcd_ops);
	if (IS_ERR(p->lcd))
		return dev_err_probe(dev, PTR_ERR(p->lcd),
				     "%s: failed to register LCD device\n",
				     desc->name);
	p->lcd->props.max_contrast = desc->max_contrast;

	return dgl_pipe_init(dgl, p);
}

static int dgl_probe(struct spi_device *spi)
{
	struct device *dev = &spi->dev;
	const struct dgl_device_desc *desc;
	struct dgl_device *dgl;
	struct drm_device *drm;
	unsigned int i;
	int ret;

	desc = spi_get_device_match_data(spi);
	if (!desc)
		return -ENODEV;

	/* The vendor SPI driver accepts mode 0 only; all traffic is 8-bit. */
	spi->mode = SPI_MODE_0;
	spi->bits_per_word = 8;
	ret = spi_setup(spi);
	if (ret)
		return dev_err_probe(dev, ret, "Failed to set up SPI device\n");

	dgl = devm_drm_dev_alloc(dev, &dgl_drm_driver, struct dgl_device, drm);
	if (IS_ERR(dgl))
		return PTR_ERR(dgl);
	drm = &dgl->drm;
	dgl->num_panels = desc->num_panels;

	ret = devm_mutex_init(dev, &dgl->lock);
	if (ret)
		return ret;

	/* Vendor name "lcda0": low for commands, high for pixel data. */
	dgl->dc = devm_gpiod_get(dev, "dc", GPIOD_OUT_LOW);
	if (IS_ERR(dgl->dc))
		return dev_err_probe(dev, PTR_ERR(dgl->dc),
				     "Failed to get D/C GPIO\n");

	ret = drmm_mode_config_init(drm);
	if (ret)
		return ret;

	drm->mode_config.funcs = &dgl_mode_config_funcs;
	drm->mode_config.min_width = INT_MAX;
	drm->mode_config.min_height = INT_MAX;
	drm->mode_config.preferred_depth = 24;

	for (i = 0; i < desc->num_panels; i++) {
		const struct drm_display_mode *mode = &desc->panels[i]->mode;
		const int width = mode->hdisplay;
		const int height = mode->vdisplay;
		struct spi_device *panel_spi = spi;

		if (i) {
			u8 cs;

			if (spi->num_chipselect <= i)
				return dev_err_probe(dev, -EINVAL,
						     "Missing chip select for %s\n",
						     desc->panels[i]->name);
			/*
			 * NOTE: the SPI core expects this to be called while
			 * the controller's add_lock is held, which is only
			 * the case when the device is probed as it is added.
			 */
			cs = spi_get_chipselect(spi, i);
			panel_spi = devm_spi_new_ancillary_device(spi, cs);
			if (IS_ERR(panel_spi))
				return PTR_ERR(panel_spi);
		}

		ret = dgl_panel_init(dgl, i, desc->panels[i], panel_spi);
		if (ret)
			return ret;

		drm->mode_config.min_width = min(drm->mode_config.min_width, width);
		drm->mode_config.max_width = max(drm->mode_config.max_width, width);
		drm->mode_config.min_height = min(drm->mode_config.min_height, height);
		drm->mode_config.max_height = max(drm->mode_config.max_height, height);
	}

	/* The "backlight" phandle lights the main panel. */
	dgl->panels[0].backlight = devm_of_find_backlight(dev);
	if (IS_ERR(dgl->panels[0].backlight))
		return dev_err_probe(dev, PTR_ERR(dgl->panels[0].backlight),
				     "Failed to get backlight\n");

	if (desc->num_panels > 1) {
		ret = dgl_dss_bl_init(dgl, &dgl->panels[1]);
		if (ret)
			return ret;
	}

	drm_mode_config_reset(drm);

	ret = drm_dev_register(drm, 0);
	if (ret)
		return ret;

	spi_set_drvdata(spi, drm);

	drm_client_setup(drm, NULL);

	return 0;
}

static void dgl_remove(struct spi_device *spi)
{
	struct drm_device *drm = spi_get_drvdata(spi);

	drm_dev_unplug(drm);
	drm_atomic_helper_shutdown(drm);
}

static void dgl_shutdown(struct spi_device *spi)
{
	drm_atomic_helper_shutdown(spi_get_drvdata(spi));
}

static const struct of_device_id dgl_of_match[] = {
	{ .compatible = "digium,d40-lcd", .data = &dgl_d40_desc },
	{ .compatible = "digium,d70-lcd", .data = &dgl_d70_desc },
	{ }
};
MODULE_DEVICE_TABLE(of, dgl_of_match);

static const struct spi_device_id dgl_spi_id[] = {
	{ "d40-lcd", (kernel_ulong_t)&dgl_d40_desc },
	{ "d70-lcd", (kernel_ulong_t)&dgl_d70_desc },
	{ }
};
MODULE_DEVICE_TABLE(spi, dgl_spi_id);

static struct spi_driver dgl_spi_driver = {
	.driver = {
		.name = "digium-gray-lcd",
		.of_match_table = dgl_of_match,
	},
	.id_table = dgl_spi_id,
	.probe = dgl_probe,
	.remove = dgl_remove,
	.shutdown = dgl_shutdown,
};
module_spi_driver(dgl_spi_driver);

MODULE_DESCRIPTION("Digium D40/D45/D50/D70 grayscale LCD DRM driver");
MODULE_LICENSE("GPL");
