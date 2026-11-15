// SPDX-License-Identifier: GPL-2.0-only
/*
 * B53 bus glue for the Ethernet switch core integrated in Broadcom BCMRING
 * SoCs (BCM11107/11109/11170/11188/11189 family).
 *
 * Written from register behavior recovered by static analysis of vendor
 * binaries and from measurements on the phones.
 *
 * The switch core uses the ordinary Broadcom "Robo" page/register numbering,
 * but it is neither behind SPI/MDIO nor laid out like the BCM63xx MMIO
 * switches handled by b53_mmap.c:
 *
 *   - every (page, reg) pair owns an 8 byte slot at
 *         core + (page << 11) + (reg << 3)
 *   - registers up to 32 bits wide are read and written as one little-endian
 *     32 bit word at the start of the slot
 *   - 48 and 64 bit registers use both words of the slot (low word first)
 *
 * A second, small "wrapper" block next to the core holds the readiness flag,
 * the PHY power-down bits, an indirect MII accessor for the integrated PHYs,
 * and the IMP speed selection.  This driver is the only owner of that block:
 * the integrated PHYs are reached through the b53 phy_read16/phy_write16
 * hooks (i.e., they sit on the DSA user MDIO bus at address = port number;
 * there is no separate MDIO bus driver), and the IMP speed register is
 * maintained here as well, so the conduit Ethernet driver never touches the
 * wrapper.
 *
 * Measured on a Digium D60 (BCM11189), a D40, and a D50 (both BCM11170)
 * under the stock firmware, with identical results: the device ID register
 * 02:30 reads 0x1107, which no b53 chip table entry matches, so the chip
 * entry is forced (see b53_bcmring_of_table); the Broadcom header register
 * 02:03 (reads 3) and the IMP override register 00:0e exist; the port VLAN
 * masks of page 0x31 exist for ports 0 to 5 and 8 (0x1ff each); the IMP port
 * reports 1000 Mb/s in the speed status register on these 10/100 SoCs too;
 * the wrapper IMP speed register reads 0x41 with the LAN port linked at 100
 * Mb/s; and the PHY registers read the same through the wrapper's accessor
 * and directly at pages 0x10 and 0x11 (b53's default path).  The accessor is
 * kept because it is the only path the vendor writes through.  The direct
 * pages are a live MDIO path for reads, not a
 * shadow copy: a direct read of 0x10:(2 * 0x1a) cleared the PHY's
 * clear-on-read interrupt status.
 *
 * Further measurements on a stock D60 (a D40 agreed where it was read).
 * Absent pages and registers read zero with no bus
 * residue, so a zero read never proves that a register exists.
 *
 *   - Ports: 00:06, 00:07, 31:0c, and 31:0e read zero while the same
 *     registers of ports 2 to 5 read reset values, i.e., the core has port
 *     slots 0 to 5 and 8 only.  b53's writes for ports 6 and 7 (its
 *     BCM53115 entry has nine port slots) go to absent slots.
 *   - MIB: the counter layout of pages 0x20 and 0x28 was verified against
 *     b53_mibs[] by counters moving as predicted under known traffic.
 *     Offsets 0x0c and 0xc4 also count on this core; b53 does not export
 *     them.
 *   - EEE: page 0x92 reads zero in all 128 slots (both words), so the EEE
 *     block is taken to be absent (inferred).  b53 claims EEE support for
 *     its BCM53115 entry and would write 92:00 from ethtool's set_eee and
 *     after a link-up with EEE negotiated; see b53_bcmring_support_eee().
 *   - No vendor code touches 00:21 bits 6 and 7, 00:22, 00:32/34/36,
 *     00:3c, 02:00 bits 0 and 5, or 42:20/28, and they read zero under the
 *     stock firmware.  b53 writes all of them during setup.  Under this
 *     driver on a D60 all of them hold what b53 wrote, except 00:22 (stays
 *     zero) and the EAP registers, which hold it in an unusual place; see
 *     b53_bcmring_report_unproven().
 */

#include <linux/bitfield.h>
#include <linux/bits.h>
#include <linux/brcmphy.h>
#include <linux/clk.h>
#include <linux/io.h>
#include <linux/iopoll.h>
#include <linux/kernel.h>
#include <linux/mfd/syscon.h>
#include <linux/module.h>
#include <linux/mutex.h>
#include <linux/of.h>
#include <linux/platform_data/b53.h>
#include <linux/phylink.h>
#include <linux/platform_device.h>
#include <linux/regmap.h>
#include <linux/reset.h>

#include "b53_priv.h"

/* Switch core: slot addressing */
#define ESW_PAGE_SHIFT			11
#define ESW_REG_SHIFT			3
#define ESW_SLOT(page, reg) \
	(((u32)(page) << ESW_PAGE_SHIFT) | ((u32)(reg) << ESW_REG_SHIFT))

/*
 * Wrapper block registers.
 *
 * ESWW_CTRL: bits 17 and 21 toggle continuously (the word was sampled as
 * 0xc088004d, 0xc08a004d, 0xc0a8004d, and 0xc0aa004d), with no relation to
 * LAN traffic, so never compare the whole word or cache it.  The ready bit
 * was set in all 3000 samples taken on two phones.  A read-modify-write
 * writes bits 17 and 21 back as sampled, as the vendor's power-down helper
 * does.
 */
#define ESWW_CTRL			0x00
#define  ESWW_CTRL_PHY_PD_MASK		GENMASK(16, 13)
#define  ESWW_CTRL_PHY_PD(port)		((5U << (13 + (port))) & ESWW_CTRL_PHY_PD_MASK)
#define  ESWW_CTRL_READY		BIT(23)
#define ESWW_MII_ADDR			0x08
#define  ESWW_MII_ADDR_VAL(page, reg)	(((page) << 8) | (((reg) & 0x7f) << 1))
#define ESWW_MII_WDATA			0x10
#define ESWW_MII_RDATA			0x18
#define ESWW_MII_CMD			0x20
#define  ESWW_MII_CMD_BUSY		BIT(3)
#define  ESWW_MII_CMD_READ		0x0e
#define  ESWW_MII_CMD_WRITE		0x0d
/*
 * For documentation only; there is no interrupt support here.  The status
 * word reads 0x30 and the mask word 0 on two stock phones.  The vendor's
 * accessors for the two words have no caller in any vendor module or
 * library, the layout is unknown, and the PHYs' own interrupt mask register
 * reads 0xffff (all masked).  PHY state is polled.
 */
#define ESWW_PHY_IRQ_STATUS		0x28
#define ESWW_PHY_IRQ_MASK		0x30
/*
 * The vendor rewrites the Rx field from the LAN port speed in its one-second
 * status work and nowhere else (static analysis); live it reads 0x41 with the
 * LAN port at 100 Mb/s.  The glue does the same from the LAN port's link-up
 * hook.
 */
#define ESWW_IMP_SPEED			0x90
#define  ESWW_IMP_SPEED_RX		GENMASK(2, 0)	/* vendor: follows the LAN port */
#define  ESWW_IMP_SPEED_TX		GENMASK(6, 4)	/* vendor: always 1000 */
#define  ESWW_IMP_SPEED_100		1
#define  ESWW_IMP_SPEED_10		2
#define  ESWW_IMP_SPEED_1000		4

/* The MII accessor reaches the integrated PHY of port n as "page" 0x10 + n */
#define ESW_INT_PHY_PAGE(port)		(0x10 + (port))

/* Only ports 0 and 1 have a PHY; port 8 is the internal IMP link */
#define ESW_NUM_PHY_PORTS		2

/* The vendor MAC driver calls core port 0 the LAN (uplink) port */
#define ESW_LAN_PORT			0

/*
 * Page 0x34 register 0x06 (B53_VLAN_CTRL5) bit 0: set by the vendor MAC
 * driver before it configures the Tx FIFO.  Not a b53-defined bit; believed
 * to make the switch accept frames from the CPU port without
 * a valid FCS, which is what the FIFO bridge delivers: the vendor driver
 * sends four bytes it never wrote in the FCS position.  b53 only
 * read-modify-writes this register and changes bits 2 and 3 alone, so the
 * bit survives b53_enable_vlan().
 */
#define ESW_VLAN_CTRL5_IMP_BIT0		BIT(0)

/**
 * struct b53_bcmring_soc - per-SoC properties of the switch core
 * @chip_id: b53 chip table entry to use (the core's own ID is not in it)
 * @gigabit: the integrated PHYs may run 1000BASE-T.  The vendor kernel
 *	decides this per SoC from its capability table and, where the
 *	capability is absent, clears the 1000BASE-T advertisement of both PHYs.
 */
struct b53_bcmring_soc {
	u32 chip_id;
	bool gigabit;
};

struct b53_bcmring_priv {
	void __iomem *core;
	void __iomem *wrap;
	const struct b53_bcmring_soc *soc;
	struct regmap *chipc;	/* optional, see b53_bcmring_pre_read() */
	struct mutex mii_lock;	/* serializes all wrapper block accesses */
	/* b53's phylink MAC ops with mac_link_up wrapped, see below */
	const struct phylink_mac_ops *b53_mac_ops;
	struct phylink_mac_ops mac_ops;
	/* b53's switch ops with the EEE and PHY flag hooks replaced */
	struct dsa_switch_ops ds_ops;
};

/*
 * The vendor code reads chip-control register 0 immediately before every
 * read of a switch or wrapper register and discards the value.  Why is not
 * known.  It is not needed for a correct value: read singly from user space
 * without it, under the stock firmware of three phones, the core and wrapper
 * registers returned the values an earlier survey had read with it, and the
 * PHY pages the values the vendor's own accessor returns.  What that does
 * not cover is reads in quick succession, so it is reproduced when the
 * devicetree gives the chip-control syscon.
 */
static void b53_bcmring_pre_read(struct b53_bcmring_priv *priv)
{
	unsigned int discard;

	if (priv->chipc)
		regmap_read(priv->chipc, 0, &discard);
}

static u32 b53_bcmring_core_rd(struct b53_device *dev, u8 page, u8 reg,
			       unsigned int word)
{
	struct b53_bcmring_priv *priv = dev->priv;

	b53_bcmring_pre_read(priv);
	return readl(priv->core + ESW_SLOT(page, reg) + 4 * word);
}

static void b53_bcmring_core_wr(struct b53_device *dev, u8 page, u8 reg,
				unsigned int word, u32 val)
{
	struct b53_bcmring_priv *priv = dev->priv;

	writel(val, priv->core + ESW_SLOT(page, reg) + 4 * word);
}

static int b53_bcmring_read8(struct b53_device *dev, u8 page, u8 reg, u8 *val)
{
	*val = b53_bcmring_core_rd(dev, page, reg, 0);
	return 0;
}

static int b53_bcmring_read16(struct b53_device *dev, u8 page, u8 reg,
			      u16 *val)
{
	*val = b53_bcmring_core_rd(dev, page, reg, 0);
	return 0;
}

static int b53_bcmring_read32(struct b53_device *dev, u8 page, u8 reg,
			      u32 *val)
{
	*val = b53_bcmring_core_rd(dev, page, reg, 0);
	return 0;
}

/*
 * The per-port EAP configuration registers (42:20 + 8 * port) are 64 bits
 * wide in b53's register map: an address in bits 47:0 and the control
 * fields, among them the EAP mode in bits 52:51, in the upper half.  On
 * this core the slot does not behave like the other 64-bit registers (MIB
 * counters and ARL entries, which are low word first in two words).
 * Measured on a D60 under this driver (read through /dev/mem):
 *
 *   - after b53 wrote 0 to word 0 and 0x00180000 (mode 3) to word 1, word 0
 *     read 0x00180000 and word 1 read 0;
 *   - after b53 then wrote 0x00180000 to word 0 and 0 to word 1 (mode 0,
 *     with the value it had read back as the "low half"), both words read 0.
 *
 * So the last value written to word 1 is what word 0 returns, word 1 itself
 * reads zero, and a value written to word 0 does not stay.  The slot is
 * therefore treated as one 32-bit register that holds the upper half of
 * b53's layout: written through word 1 (after a zero to word 0, the sequence
 * the measurement covers) and read from word 0.  The address field is not
 * representable and reads as zero; b53 never sets it.
 *
 * Not known: whether the core acts on the mode bits it stores.  Nothing in
 * this driver depends on that: standalone ports are kept apart by their port
 * masks, and 802.1X pass-through is done in software by the bridge (its
 * group forwarding mask), on frames the switch hands to the CPU port.
 */
static bool b53_bcmring_is_eap_conf(u8 page, u8 reg)
{
	return page == B53_EAP_PAGE && reg >= B53_PORT_EAP_CONF(0) &&
	       reg <= B53_PORT_EAP_CONF(B53_CPU_PORT) && !(reg & 7);
}

static int b53_bcmring_read64(struct b53_device *dev, u8 page, u8 reg,
			      u64 *val)
{
	u32 lo, hi;

	lo = b53_bcmring_core_rd(dev, page, reg, 0);
	hi = b53_bcmring_core_rd(dev, page, reg, 1);
	if (b53_bcmring_is_eap_conf(page, reg))
		*val = (u64)(lo | hi) << 32;
	else
		*val = ((u64)hi << 32) | lo;
	return 0;
}

static int b53_bcmring_read48(struct b53_device *dev, u8 page, u8 reg,
			      u64 *val)
{
	int ret = b53_bcmring_read64(dev, page, reg, val);

	*val &= GENMASK_ULL(47, 0);
	return ret;
}

static int b53_bcmring_write8(struct b53_device *dev, u8 page, u8 reg, u8 val)
{
	b53_bcmring_core_wr(dev, page, reg, 0, val);
	return 0;
}

static int b53_bcmring_write16(struct b53_device *dev, u8 page, u8 reg,
			       u16 val)
{
	b53_bcmring_core_wr(dev, page, reg, 0, val);
	return 0;
}

static int b53_bcmring_write32(struct b53_device *dev, u8 page, u8 reg,
			       u32 val)
{
	b53_bcmring_core_wr(dev, page, reg, 0, val);
	return 0;
}

static int b53_bcmring_write64(struct b53_device *dev, u8 page, u8 reg,
			       u64 val)
{
	if (b53_bcmring_is_eap_conf(page, reg))
		val &= GENMASK_ULL(63, 32);
	b53_bcmring_core_wr(dev, page, reg, 0, lower_32_bits(val));
	b53_bcmring_core_wr(dev, page, reg, 1, upper_32_bits(val));
	return 0;
}

static int b53_bcmring_write48(struct b53_device *dev, u8 page, u8 reg,
			       u64 val)
{
	return b53_bcmring_write64(dev, page, reg, val & GENMASK_ULL(47, 0));
}

static int b53_bcmring_mii_wait(struct b53_bcmring_priv *priv)
{
	u32 cmd;

	b53_bcmring_pre_read(priv);
	return readl_poll_timeout(priv->wrap + ESWW_MII_CMD, cmd,
				  !(cmd & ESWW_MII_CMD_BUSY), 10, 10000);
}

static int b53_bcmring_phy_read16(struct b53_device *dev, int addr, int reg,
				  u16 *val)
{
	struct b53_bcmring_priv *priv = dev->priv;
	int ret;

	if (addr < 0 || addr >= ESW_NUM_PHY_PORTS) {
		*val = 0xffff;
		return 0;
	}

	mutex_lock(&priv->mii_lock);
	writel(ESWW_MII_ADDR_VAL(ESW_INT_PHY_PAGE(addr), reg),
	       priv->wrap + ESWW_MII_ADDR);
	writel(ESWW_MII_CMD_READ, priv->wrap + ESWW_MII_CMD);
	ret = b53_bcmring_mii_wait(priv);
	if (!ret) {
		b53_bcmring_pre_read(priv);
		*val = readl(priv->wrap + ESWW_MII_RDATA);
	}
	mutex_unlock(&priv->mii_lock);

	return ret;
}

static int b53_bcmring_phy_write16(struct b53_device *dev, int addr, int reg,
				   u16 val)
{
	struct b53_bcmring_priv *priv = dev->priv;
	int ret;

	if (addr < 0 || addr >= ESW_NUM_PHY_PORTS)
		return 0;

	mutex_lock(&priv->mii_lock);
	writel(ESWW_MII_ADDR_VAL(ESW_INT_PHY_PAGE(addr), reg),
	       priv->wrap + ESWW_MII_ADDR);
	writel(val, priv->wrap + ESWW_MII_WDATA);
	writel(ESWW_MII_CMD_WRITE, priv->wrap + ESWW_MII_CMD);
	ret = b53_bcmring_mii_wait(priv);
	mutex_unlock(&priv->mii_lock);

	return ret;
}

static void b53_bcmring_phy_power(struct b53_device *dev, int port, bool on)
{
	struct b53_bcmring_priv *priv = dev->priv;
	u32 ctrl;

	if (port < 0 || port >= ESW_NUM_PHY_PORTS)
		return;

	/* reg_mutex is not held by the callers; the MII lock is ours */
	mutex_lock(&priv->mii_lock);
	b53_bcmring_pre_read(priv);
	ctrl = readl(priv->wrap + ESWW_CTRL);
	if (on)
		ctrl &= ~ESWW_CTRL_PHY_PD(port);
	else
		ctrl |= ESWW_CTRL_PHY_PD(port);
	writel(ctrl, priv->wrap + ESWW_CTRL);
	mutex_unlock(&priv->mii_lock);
}

static void b53_bcmring_phy_enable(struct b53_device *dev, int port)
{
	b53_bcmring_phy_power(dev, port, true);
}

static void b53_bcmring_phy_disable(struct b53_device *dev, int port)
{
	b53_bcmring_phy_power(dev, port, false);
}

static void b53_bcmring_phylink_get_caps(struct b53_device *dev, int port,
					 struct phylink_config *config)
{
	struct b53_bcmring_priv *priv = dev->priv;

	/* Ports 0/1 use integrated PHYs, port 8 is an on-chip link */
	__set_bit(PHY_INTERFACE_MODE_INTERNAL, config->supported_interfaces);

	/*
	 * The integrated PHYs have no devicetree nodes (they sit on the DSA
	 * user MDIO bus), so "max-speed" cannot be given there.  Keep phylink
	 * from offering 1000BASE-T on the SoCs without that capability: the
	 * PHY's status register announces extended status (BMSR 0x796d) and
	 * its extended status register claims 1000BASE-T full and half
	 * duplex (register 15 = 0x3000, measured on both ports of a BCM11189
	 * and of two BCM11170s), so phylib's generic auto-negotiation code
	 * (used by the PHY driver entry for this core and by the generic PHY
	 * driver alike) would advertise gigabit.  With the modes masked here
	 * it writes zero to the 1000BASE-T advertisement bits instead, which
	 * is what the vendor kernel does on these SoCs (register 9 reads 0
	 * there).
	 */
	if (port < ESW_NUM_PHY_PORTS && !priv->soc->gigabit)
		config->mac_capabilities &= ~MAC_1000;
}

static const struct b53_io_ops b53_bcmring_ops = {
	.read8 = b53_bcmring_read8,
	.read16 = b53_bcmring_read16,
	.read32 = b53_bcmring_read32,
	.read48 = b53_bcmring_read48,
	.read64 = b53_bcmring_read64,
	.write8 = b53_bcmring_write8,
	.write16 = b53_bcmring_write16,
	.write32 = b53_bcmring_write32,
	.write48 = b53_bcmring_write48,
	.write64 = b53_bcmring_write64,
	.phy_read16 = b53_bcmring_phy_read16,
	.phy_write16 = b53_bcmring_phy_write16,
	.phy_enable = b53_bcmring_phy_enable,
	.phy_disable = b53_bcmring_phy_disable,
	.phylink_get_caps = b53_bcmring_phylink_get_caps,
};

static int b53_bcmring_imp_speed_code(u32 mbps)
{
	switch (mbps) {
	case 10:
		return ESWW_IMP_SPEED_10;
	case 100:
		return ESWW_IMP_SPEED_100;
	case 1000:
		return ESWW_IMP_SPEED_1000;
	default:
		return -EINVAL;
	}
}

/*
 * Speed of the internal switch <-> CPU FIFO link.  The register has one field
 * per direction.  The vendor MAC driver (ethStatusWork) keeps the "Tx" field
 * at 1000 and rewrites the "Rx" field with the negotiated speed of the LAN
 * port whenever that changes (its log text: "Changing internal port Rx
 * speed"); a stock D40, D50, and D60 with the LAN port at 100 Mb/s
 * accordingly read 0x41.
 * Whether traffic fails when the Rx field does not match the LAN speed is not
 * known, so the vendor behavior is reproduced: the Tx field follows the CPU
 * port's fixed link, the Rx field follows the LAN port.
 */
static void b53_bcmring_imp_speed_set(struct b53_device *dev, u32 mask,
				      int speed)
{
	struct b53_bcmring_priv *priv = dev->priv;
	int code = b53_bcmring_imp_speed_code(speed);
	u32 val;

	if (code < 0) {
		dev_warn(dev->dev, "unsupported IMP speed %d\n", speed);
		return;
	}

	mutex_lock(&priv->mii_lock);
	b53_bcmring_pre_read(priv);
	val = readl(priv->wrap + ESWW_IMP_SPEED);
	val &= ~mask;
	val |= field_prep(mask, code);
	writel(val, priv->wrap + ESWW_IMP_SPEED);
	mutex_unlock(&priv->mii_lock);
}

/*
 * b53 has no bus-glue hook for link changes, so wrap its phylink
 * mac_link_up: run the b53 handler, then update the wrapper IMP speed
 * register.
 */
static void b53_bcmring_mac_link_up(struct phylink_config *config,
				    struct phy_device *phydev,
				    unsigned int mode,
				    phy_interface_t interface,
				    int speed, int duplex,
				    bool tx_pause, bool rx_pause)
{
	struct dsa_port *dp = dsa_phylink_to_port(config);
	struct b53_device *dev = dp->ds->priv;
	struct b53_bcmring_priv *priv = dev->priv;

	priv->b53_mac_ops->mac_link_up(config, phydev, mode, interface, speed,
				       duplex, tx_pause, rx_pause);

	if (dsa_port_is_cpu(dp))
		b53_bcmring_imp_speed_set(dev, ESWW_IMP_SPEED_TX, speed);
	else if (dp->index == ESW_LAN_PORT)
		b53_bcmring_imp_speed_set(dev, ESWW_IMP_SPEED_RX, speed);
}

/*
 * The EEE control page (0x92) reads zero throughout on this core and is
 * taken to be absent, while b53_support_eee() is true for the BCM53115 chip
 * entry used here.  Returning false makes the DSA core refuse ethtool's
 * get_eee/set_eee before b53_set_mac_eee() can write 92:00.  The other
 * writer, b53_eee_init() on link-up, only acts when the PHY has negotiated
 * EEE, which these PHYs cannot (they have no MMD registers).
 */
static bool b53_bcmring_support_eee(struct dsa_switch *ds, int port)
{
	return false;
}

/*
 * Tell the PHY driver which SoCs may run 1000BASE-T: the PHY itself claims
 * gigabit on all of them, and one vendor fixup applies to the gigabit SoCs
 * only.
 */
static u32 b53_bcmring_get_phy_flags(struct dsa_switch *ds, int port)
{
	struct b53_device *dev = ds->priv;
	struct b53_bcmring_priv *priv = dev->priv;

	if (port < ESW_NUM_PHY_PORTS && priv->soc->gigabit)
		return PHY_BRCM_BCMRING_GIGABIT;

	return 0;
}

/*
 * Run once after b53 has set the switch up.  The registers below are used
 * by b53 but no vendor code touches them, and they read zero under the stock
 * firmware, which is also how an absent register reads.  b53's setup writes
 * a non-zero value to each, so this readback shows which exist: a field that
 * still reads zero is absent on this core.  Expected after b53's setup:
 * 00:21 = 0xc1, 00:22 with bit 6 set, the three flood masks and the
 * learning-disable mask with the bits of the user and CPU ports, and mode 3
 * in each user port's EAP register (42:20, 42:28; see
 * b53_bcmring_is_eap_conf() for how this core presents it).  Seen on a D60:
 * all as expected except 00:22, which stays zero (absent; its "dumb
 * forwarding" bit only concerns unmanaged mode).  02:00 is printed for its
 * MIB bits (0 and 5), which b53 leaves clear, so its value proves nothing
 * about them.
 */
static void b53_bcmring_report_unproven(struct b53_device *bdev)
{
	static const struct {
		u8 reg;
		u16 expect;	/* bits b53's setup should have set */
		const char *name;
	} ctrl[] = {
		{ B53_IP_MULTICAST_CTRL, B53_UC_FWD_EN | B53_MC_FWD_EN,
		  "flood enables" },
		{ B53_SWITCH_CTRL, B53_MII_DUMB_FWDG_EN, "switch control" },
		{ B53_UC_FLOOD_MASK, BIT(B53_CPU_PORT), "unicast flood mask" },
		{ B53_MC_FLOOD_MASK, BIT(B53_CPU_PORT), "multicast flood mask" },
		{ B53_IPMC_FLOOD_MASK, BIT(B53_CPU_PORT),
		  "IP multicast flood mask" },
		{ B53_DIS_LEARNING, BIT(ESW_LAN_PORT), "learning disable" },
	};
	struct device *dev = bdev->dev;
	unsigned int i;
	u32 val;

	for (i = 0; i < ARRAY_SIZE(ctrl); i++) {
		val = b53_bcmring_core_rd(bdev, B53_CTRL_PAGE, ctrl[i].reg, 0);
		dev_info(dev, "after setup: 00:%02x (%s) = %#06x%s\n",
			 ctrl[i].reg, ctrl[i].name, val & 0xffff,
			 (val & ctrl[i].expect) == ctrl[i].expect ? "" :
			 ", expected bits missing: register absent?");
	}

	val = b53_bcmring_core_rd(bdev, B53_MGMT_PAGE, B53_GLOBAL_CONFIG, 0);
	dev_info(dev, "after setup: 02:00 (global config) = %#04x\n",
		 val & 0xff);

	for (i = 0; i < ESW_NUM_PHY_PORTS; i++) {
		u64 eap;

		if (!dsa_is_user_port(bdev->ds, i))
			continue;
		/* Standalone user ports are in mode 3 after setup. */
		b53_bcmring_read64(bdev, B53_EAP_PAGE, B53_PORT_EAP_CONF(i),
				   &eap);
		dev_info(dev, "after setup: 42:%02x (EAP, port %u) = %#010x, mode %llu%s\n",
			 B53_PORT_EAP_CONF(i), i, upper_32_bits(eap),
			 (eap & EAP_MODE_MASK) >> EAP_MODE_SHIFT,
			 (eap & EAP_MODE_MASK) == EAP_MODE_MASK ? "" :
			 ", expected mode 3");
	}
}

static int b53_bcmring_parse_ports(struct device *dev,
				   struct b53_platform_data *pdata)
{
	struct device_node *np = dev->of_node;
	struct device_node *ports, *port;
	u32 reg;

	ports = of_get_child_by_name(np, "ethernet-ports");
	if (!ports)
		ports = of_get_child_by_name(np, "ports");
	if (!ports)
		return dev_err_probe(dev, -EINVAL, "no ports child node\n");

	for_each_available_child_of_node(ports, port) {
		if (of_property_read_u32(port, "reg", &reg))
			continue;
		if (reg >= B53_N_PORTS)
			continue;

		pdata->enabled_ports |= BIT(reg);
	}
	of_node_put(ports);

	return 0;
}

static int b53_bcmring_probe(struct platform_device *pdev)
{
	struct device *dev = &pdev->dev;
	struct b53_platform_data *pdata;
	struct b53_bcmring_priv *priv;
	struct clk_bulk_data *clks;
	struct reset_control *rst;
	struct b53_device *bdev;
	u32 ctrl, id, rev, hdr, vc5;
	int ret;

	priv = devm_kzalloc(dev, sizeof(*priv), GFP_KERNEL);
	if (!priv)
		return -ENOMEM;

	pdata = devm_kzalloc(dev, sizeof(*pdata), GFP_KERNEL);
	if (!pdata)
		return -ENOMEM;

	ret = devm_mutex_init(dev, &priv->mii_lock);
	if (ret)
		return ret;

	priv->core = devm_platform_ioremap_resource_byname(pdev, "core");
	if (IS_ERR(priv->core))
		return PTR_ERR(priv->core);

	priv->wrap = devm_platform_ioremap_resource_byname(pdev, "wrapper");
	if (IS_ERR(priv->wrap))
		return PTR_ERR(priv->wrap);

	priv->chipc = syscon_regmap_lookup_by_phandle_optional(dev->of_node,
							       "brcm,chipc");
	if (IS_ERR(priv->chipc))
		return dev_err_probe(dev, PTR_ERR(priv->chipc),
				     "bad chip-control syscon\n");

	/*
	 * The vendor kernel ungates the switch through chip-control (bus
	 * interface gate bit 8, ESW clock output enable), sets the ESW clock
	 * to 100 MHz, and (in the "eth reset" path only) pulses two
	 * chip-control soft resets.  Those belong to the SoC clock and reset
	 * provider; here all listed clocks are enabled (the rate comes from
	 * assigned-clock-rates) and all listed resets are released.  The
	 * resets are not asserted again on unbind: the conduit's FIFO bridge
	 * lives in the same block.
	 */
	ret = devm_clk_bulk_get_all_enabled(dev, &clks);
	if (ret < 0)
		return dev_err_probe(dev, ret, "failed to enable clocks\n");

	rst = devm_reset_control_array_get_optional_exclusive(dev);
	if (IS_ERR(rst))
		return dev_err_probe(dev, PTR_ERR(rst), "no switch reset\n");
	ret = reset_control_deassert(rst);
	if (ret)
		return dev_err_probe(dev, ret, "failed to release reset\n");

	/* Vendor: ~1.25 ms settle time, then poll the ready flag */
	usleep_range(1250, 2500);
	b53_bcmring_pre_read(priv);
	ret = readl_poll_timeout(priv->wrap + ESWW_CTRL, ctrl,
				 ctrl & ESWW_CTRL_READY, 100, 50000);
	if (ret)
		return dev_err_probe(dev, ret, "switch core not ready\n");

	priv->soc = device_get_match_data(dev);
	if (!priv->soc)
		return -ENODEV;
	pdata->chip_id = priv->soc->chip_id;

	ret = b53_bcmring_parse_ports(dev, pdata);
	if (ret)
		return ret;

	bdev = b53_switch_alloc(dev, &b53_bcmring_ops, priv);
	if (!bdev)
		return -ENOMEM;

	bdev->pdata = pdata;
	platform_set_drvdata(pdev, bdev);

	/*
	 * The core's own ID (0x1107 on a BCM11189) is not in b53's chip
	 * table; report what was read and which entry is used instead.
	 *
	 * Also report two registers as found, before anything here or in b53
	 * writes them.  No vendor code writes the Broadcom header register
	 * (02:03), yet it reads 3 under the stock firmware, so that is
	 * presumably its reset value; since the vendor boot loader does not
	 * touch the switch, the value found here settles it.
	 */
	id = b53_bcmring_core_rd(bdev, B53_MGMT_PAGE, B53_DEVICE_ID, 0);
	rev = b53_bcmring_core_rd(bdev, B53_MGMT_PAGE, B53_REV_ID, 0);
	hdr = b53_bcmring_core_rd(bdev, B53_MGMT_PAGE, B53_BRCM_HDR, 0);
	vc5 = b53_bcmring_core_rd(bdev, B53_VLAN_PAGE, B53_VLAN_CTRL5, 0);
	dev_info(dev,
		 "switch core ID %#06x rev %#04x, handled as b53 chip ID %#x%s\n",
		 id, rev & 0xff, pdata->chip_id,
		 priv->soc->gigabit ? "" : ", PHYs limited to 100 Mb/s");
	dev_info(dev, "as found: header control %#04x, VLAN control 5 %#04x\n",
		 hdr & 0xff, vc5 & 0xff);

	/* b53 only ever read-modify-writes this register, so the bit stays. */
	b53_bcmring_core_wr(bdev, B53_VLAN_PAGE, B53_VLAN_CTRL5, 0,
			    vc5 | ESW_VLAN_CTRL5_IMP_BIT0);

	priv->b53_mac_ops = bdev->ds->phylink_mac_ops;
	priv->mac_ops = *priv->b53_mac_ops;
	priv->mac_ops.mac_link_up = b53_bcmring_mac_link_up;
	bdev->ds->phylink_mac_ops = &priv->mac_ops;

	priv->ds_ops = *bdev->ds->ops;
	priv->ds_ops.support_eee = b53_bcmring_support_eee;
	priv->ds_ops.get_phy_flags = b53_bcmring_get_phy_flags;
	bdev->ds->ops = &priv->ds_ops;

	ret = b53_switch_register(bdev);
	if (ret)
		return ret;

	/*
	 * A single-switch tree is set up inside dsa_register_switch(), so
	 * b53's writes have happened by now.
	 */
	b53_bcmring_report_unproven(bdev);

	return 0;
}

static void b53_bcmring_remove(struct platform_device *pdev)
{
	struct b53_device *bdev = platform_get_drvdata(pdev);

	if (bdev)
		b53_switch_remove(bdev);
}

static void b53_bcmring_shutdown(struct platform_device *pdev)
{
	struct b53_device *bdev = platform_get_drvdata(pdev);

	if (bdev)
		b53_switch_shutdown(bdev);

	platform_set_drvdata(pdev, NULL);
}

/*
 * The core's own ID register (page 0x02, reg 0x30) reads 0x1107 on a
 * BCM11189, which b53_switch_detect() would reject, so the chip table entry
 * is forced through platform data instead.  BCM53115 is the closest
 * register-compatible entry (ARL/VLAN table access layout, 4 ARL bins, IMP
 * on port 8, 4-byte Broadcom tag after the source address).  Unlike the
 * BCM5395 entry, which has the same layout, it makes b53 leave the soft
 * reset register (00:79) alone; no vendor code touches that register.
 *
 * Gigabit capability per SoC follows the vendor kernel's capability table
 * (BCM11109 and BCM11188: GMII capability on both ports; BCM11170 and
 * BCM11189: none).  The generic compatible alone gets the mode every SoC
 * supports.
 */
static const struct b53_bcmring_soc b53_bcmring_soc_100 = {
	.chip_id = BCM53115_DEVICE_ID,
	.gigabit = false,
};

static const struct b53_bcmring_soc b53_bcmring_soc_1000 = {
	.chip_id = BCM53115_DEVICE_ID,
	.gigabit = true,
};

static const struct of_device_id b53_bcmring_of_table[] = {
	{ .compatible = "brcm,bcm11109-switch", .data = &b53_bcmring_soc_1000 },
	{ .compatible = "brcm,bcm11188-switch", .data = &b53_bcmring_soc_1000 },
	{ .compatible = "brcm,bcm11170-switch", .data = &b53_bcmring_soc_100 },
	{ .compatible = "brcm,bcm11189-switch", .data = &b53_bcmring_soc_100 },
	{ .compatible = "brcm,bcmring-switch", .data = &b53_bcmring_soc_100 },
	{ /* sentinel */ }
};
MODULE_DEVICE_TABLE(of, b53_bcmring_of_table);

static struct platform_driver b53_bcmring_driver = {
	.probe = b53_bcmring_probe,
	.remove = b53_bcmring_remove,
	.shutdown = b53_bcmring_shutdown,
	.driver = {
		.name = "b53-bcmring",
		.of_match_table = b53_bcmring_of_table,
	},
};
module_platform_driver(b53_bcmring_driver);

MODULE_DESCRIPTION("B53 switch glue for Broadcom BCMRING integrated switch");
MODULE_LICENSE("GPL");
