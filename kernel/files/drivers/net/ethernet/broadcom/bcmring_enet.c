// SPDX-License-Identifier: GPL-2.0-only
/*
 * Broadcom BCMRING (BCM1110x/1117x/1118x) embedded-switch CPU port driver.
 *
 * Written from behavior recovered from the vendor "bcmring_net" module and
 * the vendor kernel's ethHw_* helpers.  The channel programming below was
 * compared with the registers of DMA channels 2 and 3 and with the Rx
 * descriptor ring read from a D40, a D50, and a D60 running the stock
 * firmware ("measured" in the comments): control, configuration, and
 * status-address words, the ring layout, and the bridge configuration words
 * are the ones written here.
 *
 * Hardware summary:
 *
 *  - The SoC has no conventional Ethernet MAC with its own descriptor rings.
 *    The CPU reaches the embedded 2-port + IMP switch through a pair of FIFOs
 *    ("port FIFO bridge", physical 0x30480000 for Rx and 0x30490000 for Tx).
 *  - The FIFOs are serviced by two dedicated channels of the SoC's second
 *    Synopsys DesignWare AHB DMA controller (DMA1, physical 0x30300000):
 *    channel 2 for Rx and channel 3 for Tx.  The "descriptor rings" are
 *    DesignWare multi-block linked-list items (LLIs).
 *  - Rx relies on two DesignWare features the mainline dw_dmac dmaengine
 *    driver does not expose: the peripheral as flow controller with one LLI
 *    per frame, and source-status write-back (the frame status/length word is
 *    fetched from the FIFO bridge and stored in the LLI).  This driver
 *    therefore programs the two channels directly, as the vendor driver did,
 *    through its own "dmac" resource.  It owns the whole of that controller
 *    (register block, bus clock gate, the two channel interrupts); the
 *    devicetree keeps the controller's dw_dmac node disabled, because
 *    dw_dmac resets and powers down the whole controller.
 *    Moving to dmaengine would take support for the following in dw_dmac (or
 *    dw_dmac leaving channels 2 and 3 alone):
 *      - one interrupt line per channel (channel 2 = Rx, channel 3 = Tx);
 *      - Rx: device-to-memory with the peripheral as flow controller, one
 *        block per frame of unknown length (up to 2048 bytes), a circular
 *        list of blocks that is re-armed without stopping the channel, a
 *        completion callback per block, and the per-block source status
 *        word (SSTATAR fetch, CFG.SS_UPD_EN) returned with the completion,
 *        because that word is the only source of the frame length;
 *      - Tx: memory-to-device scatter list in which consecutive entries go
 *        to two different peripheral addresses (control word, then data),
 *        one completion per list;
 *      - 64-bit transfer width on both sides with burst length 8, FIFO
 *        mode, priority 7, handshake interface 1 for Tx.
 *    The controller is built for this: the vendor's own decode of its
 *    parameter register gives a 64-bit data bus, and
 *    the per-channel parameter words report programmable widths, peripheral
 *    flow control, status write-back, and a 64-byte channel FIFO (one burst
 *    of eight 64-bit words) on channels 2 and 3.
 *  - Frames on the CPU port carry the 4-byte Broadcom tag after the source
 *    MAC address (DSA_TAG_PROTO_BRCM).  This driver is a plain DSA conduit:
 *    it never touches the tag.  It only removes the trailing FCS on Rx and
 *    supplies four placeholder FCS bytes on Tx (the bridge computes none, and
 *    the vendor hands it four bytes it never wrote).  Measured evidence that
 *    the placeholder is acceptable: on a stock D60 the IMP port's
 *    RxFCSErrors, alignment, undersize, oversize, and drop counters were all
 *    zero after more than 10,000 frames sent this way by the vendor driver.
 *    That was with page 0x34 register 0x06 bit 0 set (see the switch
 *    driver); which setting makes the switch tolerate it is not isolated.
 *  - Rx blocks are ended by the bridge, not by a count, so nothing but the
 *    switch's own frame-size limit keeps a frame inside the 2048-byte buffer.
 *    The switch must therefore never be put in jumbo mode; see
 *    BCMRING_MAX_MTU.
 *  - Everything on the switch side (switch registers, the "wrapper" control
 *    block with the ready flag, the PHY accessor, and the IMP speed
 *    register) belongs to the b53 BCMRING switch driver; this driver maps
 *    none of it.
 */

#include <linux/bitfield.h>
#include <linux/bits.h>
#include <linux/clk.h>
#include <linux/dma-mapping.h>
#include <linux/etherdevice.h>
#include <linux/if_vlan.h>
#include <linux/interrupt.h>
#include <linux/io.h>
#include <linux/iopoll.h>
#include <linux/module.h>
#include <linux/netdevice.h>
#include <linux/of.h>
#include <linux/of_net.h>
#include <linux/phylink.h>
#include <linux/platform_device.h>
#include <linux/skbuff.h>
#include <linux/spinlock.h>

#define DRV_NAME			"bcmring-enet"

/*
 * Port FIFO bridge, resource "fifo" (vendor physical base 0x30480000).
 * Verified offsets; the meaning of the configuration values is unknown, so
 * they are replayed verbatim.  The two configuration words read back
 * unchanged on two phones across traffic and a reboot: plain configuration,
 * never status.  No vendor component writes any other value.
 */
#define RXF_CFG				0x00008
#define   RXF_CFG_VENDOR		0x90040010
#define RXF_STATUS			0x00010	/* sampled by the DMAC, never by the CPU */
#define RXF_DATA			0x08000	/* read with an incrementing address */
#define TXF_CFG				0x10000
#define   TXF_CFG_VENDOR		0x90000013
#define TXF_CTRL			0x10008	/* per-frame control word */
#define TXF_STATUS			0x10010	/* sampled by the DMAC, never by the CPU */
#define TXF_DATA			0x18000	/* written with an incrementing address */

/*
 * Rx status word stored in the LLI by the DMAC.  Bits 28:13 are the frame
 * length (vendor driver).  Measured: the form
 *	BIT(31) | BIT(29) | len << 13 | ALIGN(len, 8)
 * was observed for about 60 distinct words on a D60 and a D40 under the
 * stock firmware, covering unicast, broadcast, and IPv4/IPv6 multicast
 * frames from 64 to 1518 bytes on the wire.  "len" is the wire length
 * including the FCS, plus the 4-byte Broadcom tag.  The word has no
 * cast-type or frame-type field and no sequence counter.  All of those
 * frames arrived on switch port 0; a source-port field has not been ruled
 * out for port 1.  Error bits have never been seen, so nothing is known
 * about error indications; bit 30 is the only bit never observed set.
 */
#define RXF_STATUS_LEN			GENMASK(28, 13)
#define RXF_STATUS_BYTES		GENMASK(12, 0)
#define RXF_STATUS_USUAL		(BIT(31) | BIT(29))

/*
 * Tx status word: measured 0x20000000 after every frame.  The DMAC writes it
 * to the DSTAT slot of both the control LLI and the data LLI (and DONE plus
 * the block size to CTL high of both); checking the data LLI alone, as this
 * driver does, is sufficient.
 */
#define TXF_STATUS_USUAL		BIT(29)

/* Tx per-frame control word (64-bit FIFO entry, upper half zero). */
#define TXF_CTRL_FIRST_OFS		GENMASK(2, 0)
#define TXF_CTRL_LAST_OFS		GENMASK(10, 8)
/*
 * Seen set in every live control word (17 distinct frames); the 32-bit word
 * that follows is zero.  Meaning still unknown.
 */
#define TXF_CTRL_BIT16			BIT(16)

/*
 * DesignWare AHB DMAC, resource "dmac" (vendor physical base 0x30300000).
 * Standard DW_ahb_dmac layout; every offset below except DW_STATUS_* was
 * observed in the vendor module.
 */
#define DW_CH(n)			((n) * 0x58)
#define DW_SAR				0x00
#define DW_DAR				0x08
#define DW_LLP				0x10
#define DW_CTL_LO			0x18
#define DW_CTL_HI			0x1c
#define DW_SSTATAR			0x30
#define DW_DSTATAR			0x38
#define DW_CFG_LO			0x40
#define DW_CFG_HI			0x44

#define DW_STATUS_BLOCK			0x2f0	/* from the DW layout, not from the module */
#define DW_STATUS_ERR			0x308	/* from the DW layout, not from the module */
#define DW_MASK_TFR			0x310
#define DW_MASK_BLOCK			0x318
#define DW_MASK_SRCTRAN			0x320
#define DW_MASK_DSTTRAN			0x328
#define DW_MASK_ERR			0x330
#define DW_CLEAR_TFR			0x338
#define DW_CLEAR_BLOCK			0x340
#define DW_CLEAR_SRCTRAN		0x348
#define DW_CLEAR_DSTTRAN		0x350
#define DW_CLEAR_ERR			0x358
#define DW_DMA_CFG			0x398
#define DW_CH_EN			0x3a0

/* Mask and enable registers carry a write-enable bit per channel at n + 8. */
#define DW_CH_SET(n)			(BIT((n) + 8) | BIT(n))
#define DW_CH_CLR(n)			BIT((n) + 8)

#define DW_CTLL_INT_EN			BIT(0)
#define DW_CTLH_DONE			BIT(12)

/*
 * Vendor channel programming, replayed verbatim (checked bit for bit against
 * the vendor module's dmaStart and tx functions).  Field by field:
 *
 * RX_CTL_LO 0x1a408037
 *   bit 0       INT_EN (set in every LLI, clear in the channel register)
 *   bits 3:1    DST_TR_WIDTH = 3, 64 bit
 *   bits 6:4    SRC_TR_WIDTH = 3, 64 bit
 *   bits 8:7    DINC = 0, bits 10:9 SINC = 0: both addresses increment, so
 *               the FIFO is read through its 32 KiB window, not at one address
 *   bits 13:11  DEST_MSIZE = 0
 *   bits 16:14  SRC_MSIZE = 2, bursts of 8 words (64 bytes, the FIFO depth)
 *   bits 22:20  TT_FC = 4, peripheral to memory, peripheral is flow controller
 *   bits 24:23  DMS = 0, memory on master 1
 *   bits 26:25  SMS = 1, FIFO bridge on master 2
 *   bits 28:27  LLP_DST_EN, LLP_SRC_EN
 * TX_CTL_LO 0x18901036
 *   as above except DEST_MSIZE = 2 (bursts of 8), SRC_MSIZE = 0, TT_FC = 1
 *   (memory to peripheral, DMAC is flow controller), DMS = 1 (bridge on
 *   master 2), SMS = 0 (memory on master 1); INT_EN is set on the last LLI of
 *   a chain only.
 * CFG low 0xe0: channel priority 7; hardware handshaking, active-high
 *   handshake signals, no reload.
 * CFG high: FIFO_MODE (bit 1), PROTCTL = 1 (bits 4:2), plus SS_UPD_EN
 *   (bit 6) for Rx, or DS_UPD_EN (bit 5) and DEST_PER = 1 (bits 14:11) for
 *   Tx.  Rx uses handshake interface 0 (SRC_PER = 0).
 */
#define RX_CTL_LO			0x1a408037
#define TX_CTL_LO			0x18901036
#define CH_CFG_LO_PRIO7			0x000000e0
#define RX_CFG_HI			0x00000046	/* FIFO mode, PROTCTL 1, SS_UPD_EN */
#define TX_CFG_HI			0x00000826	/* as Rx but DS_UPD_EN, DEST_PER 1 */

#define DMA_WORD			8		/* the channels move 64-bit words */

#define RX_CHAN				2
#define TX_CHAN				3

#define RX_RING_SIZE			64	/* vendor default descRx */
#define TX_RING_FRAMES			256	/* vendor default: 1024 LLIs, 2 per frame */
#define RX_BUF_SIZE			2048	/* vendor Rx buffer and block size */
#define BRCM_TAG_LEN			4

/*
 * Conduit MTU limit: 1500 bytes of payload plus the Broadcom tag, and no
 * more.  DSA derives the largest user-port MTU from it (1500), and b53
 * switches the whole switch to jumbo frames (up to 9724 bytes) as soon as the
 * CPU port MTU exceeds 1500.  A frame longer than RX_BUF_SIZE would then run
 * past the end of the Rx buffer, because the DMAC does not bound a block
 * whose flow controller is the peripheral.  802.1Q-tagged frames of 1500
 * bytes payload still pass (the stack does not count the VLAN header against
 * the MTU); with tag and FCS they are 1526 bytes on the bridge, the largest
 * frame the vendor driver is known to send.  What the bridge itself accepts
 * beyond that is unknown.
 */
#define BCMRING_MAX_MTU			(ETH_DATA_LEN + BRCM_TAG_LEN)

/* DesignWare LLI plus one word the hardware never touches. */
struct bcmring_lli {
	__le32 sar;
	__le32 dar;
	__le32 llp;
	__le32 ctl_lo;
	__le32 ctl_hi;
	__le32 sstat;
	__le32 dstat;
	__le32 unused;
};

struct bcmring_rx_buf {
	struct sk_buff *skb;
	dma_addr_t dma;
};

struct bcmring_tx_buf {
	struct sk_buff *skb;
	dma_addr_t dma;
	unsigned int map_len;
	unsigned int frame_len;
};

struct bcmring_enet {
	struct device *dev;
	struct net_device *ndev;

	void __iomem *fifo;
	phys_addr_t fifo_phys;
	void __iomem *dmac;

	struct phylink *phylink;
	struct phylink_config phylink_config;

	int rx_irq;
	int tx_irq;

	struct napi_struct rx_napi;
	struct bcmring_lli *rx_ring;
	dma_addr_t rx_ring_dma;
	struct bcmring_rx_buf rx_buf[RX_RING_SIZE];
	unsigned int rx_head;
	u32 rx_status_seen;		/* bits reported by bcmring_rx_poll() */

	struct napi_struct tx_napi;
	spinlock_t tx_lock;		/* everything below */
	struct bcmring_lli *tx_ring;
	dma_addr_t tx_ring_dma;
	__le64 *tx_ctrl;
	dma_addr_t tx_ctrl_dma;
	struct bcmring_tx_buf tx_buf[TX_RING_FRAMES];
	struct sk_buff_head tx_pending;
	unsigned int tx_inflight;
	u32 tx_status_seen;		/* bits reported by bcmring_tx_reap() */
};

/* ------------------------------------------------------------------------ */
/* DMAC channel helpers                                                      */
/* ------------------------------------------------------------------------ */

static void dw_ch_writel(struct bcmring_enet *priv, unsigned int ch, u32 reg,
			 u32 val)
{
	writel(val, priv->dmac + DW_CH(ch) + reg);
}

static void dw_irq_ack(struct bcmring_enet *priv, unsigned int ch)
{
	writel(BIT(ch), priv->dmac + DW_CLEAR_ERR);
	writel(BIT(ch), priv->dmac + DW_CLEAR_BLOCK);
}

static void dw_block_irq(struct bcmring_enet *priv, unsigned int ch, bool on)
{
	writel(on ? DW_CH_SET(ch) : DW_CH_CLR(ch), priv->dmac + DW_MASK_BLOCK);
}

static bool dw_ch_busy(struct bcmring_enet *priv, unsigned int ch)
{
	return readl(priv->dmac + DW_CH_EN) & BIT(ch);
}

static void dw_ch_stop(struct bcmring_enet *priv, unsigned int ch)
{
	u32 val;

	writel(DW_CH_CLR(ch), priv->dmac + DW_CH_EN);
	/*
	 * The vendor driver does not wait here.  The DesignWare core only
	 * drops the enable bit once the current burst has finished, so wait
	 * before the buffers are unmapped.
	 */
	if (readl_poll_timeout_atomic(priv->dmac + DW_CH_EN, val,
				      !(val & BIT(ch)), 10, 10000))
		netdev_warn(priv->ndev, "DMA channel %u did not stop\n", ch);
}

#define DW_ALL_CH_CLR			0xff00	/* write enables only: clear all */
#define DW_ALL_CH_BITS			0x00ff

static const u32 dw_irq_clear_regs[] = {
	DW_CLEAR_TFR, DW_CLEAR_BLOCK, DW_CLEAR_SRCTRAN,
	DW_CLEAR_DSTTRAN, DW_CLEAR_ERR,
};

static const u32 dw_irq_mask_regs[] = {
	DW_MASK_TFR, DW_MASK_BLOCK, DW_MASK_SRCTRAN,
	DW_MASK_DSTTRAN, DW_MASK_ERR,
};

/*
 * This driver owns the whole of DMA controller 1 (dw_dmac is not bound to
 * it), although it only uses channels 2 and 3.  The state the boot loader
 * leaves in the other six channels is unknown and nobody requests their
 * interrupt lines, so switch the controller on and then disable, mask, and
 * acknowledge all eight channels.
 */
static void bcmring_dmac_quiesce(struct bcmring_enet *priv)
{
	unsigned int i;

	writel(1, priv->dmac + DW_DMA_CFG);
	writel(DW_ALL_CH_CLR, priv->dmac + DW_CH_EN);
	for (i = 0; i < ARRAY_SIZE(dw_irq_mask_regs); i++)
		writel(DW_ALL_CH_CLR, priv->dmac + dw_irq_mask_regs[i]);
	for (i = 0; i < ARRAY_SIZE(dw_irq_clear_regs); i++)
		writel(DW_ALL_CH_BITS, priv->dmac + dw_irq_clear_regs[i]);
}

/* devm action: runs after the netdev is gone (channels already stopped). */
static void bcmring_dmac_off(void *data)
{
	struct bcmring_enet *priv = data;

	bcmring_dmac_quiesce(priv);
	writel(0, priv->dmac + DW_DMA_CFG);
}

/* Common part of the vendor's per-channel setup sequence (dmaStart). */
static void dw_ch_setup(struct bcmring_enet *priv, unsigned int ch, u32 ctl_lo,
			u32 cfg_hi, u32 sstatar, u32 dstatar)
{
	unsigned int i;

	dw_ch_writel(priv, ch, DW_CTL_LO, ctl_lo & ~DW_CTLL_INT_EN);
	dw_ch_writel(priv, ch, DW_CTL_HI, 0);
	dw_ch_writel(priv, ch, DW_CFG_LO, CH_CFG_LO_PRIO7);
	dw_ch_writel(priv, ch, DW_CFG_HI, cfg_hi);
	dw_ch_writel(priv, ch, DW_SSTATAR, sstatar);
	dw_ch_writel(priv, ch, DW_DSTATAR, dstatar);

	for (i = 0; i < ARRAY_SIZE(dw_irq_clear_regs); i++)
		writel(BIT(ch), priv->dmac + dw_irq_clear_regs[i]);
	for (i = 0; i < ARRAY_SIZE(dw_irq_mask_regs); i++)
		writel(DW_CH_CLR(ch), priv->dmac + dw_irq_mask_regs[i]);
}

/* ------------------------------------------------------------------------ */
/* Rx                                                                        */
/* ------------------------------------------------------------------------ */

static struct sk_buff *bcmring_rx_alloc(struct bcmring_enet *priv,
					dma_addr_t *dma)
{
	struct sk_buff *skb;

	/* The channel writes 64-bit words: the buffer must be 8-byte aligned. */
	skb = netdev_alloc_skb(priv->ndev, RX_BUF_SIZE + DMA_WORD);
	if (!skb)
		return NULL;
	skb_reserve(skb, PTR_ALIGN(skb->data, DMA_WORD) - skb->data);

	*dma = dma_map_single(priv->dev, skb->data, RX_BUF_SIZE,
			      DMA_FROM_DEVICE);
	if (dma_mapping_error(priv->dev, *dma)) {
		dev_kfree_skb_any(skb);
		return NULL;
	}

	return skb;
}

static void bcmring_rx_arm(struct bcmring_enet *priv, unsigned int i)
{
	struct bcmring_lli *lli = &priv->rx_ring[i];

	lli->dar = cpu_to_le32(priv->rx_buf[i].dma);
	lli->sstat = 0;
	lli->dstat = 0;
	/* Written last: clears the DONE bit the DMAC stored on completion. */
	lli->ctl_hi = cpu_to_le32(RX_BUF_SIZE);
}

static bool bcmring_rx_done(struct bcmring_enet *priv, unsigned int i)
{
	struct bcmring_lli *lli = &priv->rx_ring[i];

	return READ_ONCE(lli->sstat) &&
	       (le32_to_cpu(READ_ONCE(lli->ctl_hi)) & DW_CTLH_DONE);
}

/*
 * The LLI ring is circular and the DMAC never stops on its own, so after an
 * overrun the next completed LLI is not necessarily the one at rx_head.  Like
 * the vendor driver, look for the next completed LLI and continue from there.
 */
static bool bcmring_rx_resync(struct bcmring_enet *priv)
{
	unsigned int i, n;

	for (n = 1; n < RX_RING_SIZE; n++) {
		i = (priv->rx_head + n) % RX_RING_SIZE;
		if (bcmring_rx_done(priv, i)) {
			priv->rx_head = i;
			priv->ndev->stats.rx_fifo_errors++;
			return true;
		}
	}

	return false;
}

static int bcmring_rx_poll(struct napi_struct *napi, int budget)
{
	struct bcmring_enet *priv = container_of(napi, struct bcmring_enet,
						 rx_napi);
	struct net_device *ndev = priv->ndev;
	int done = 0;

	while (done < budget) {
		unsigned int i = priv->rx_head;
		struct bcmring_rx_buf *buf = &priv->rx_buf[i];
		struct sk_buff *skb, *new_skb;
		dma_addr_t new_dma;
		unsigned int len;
		u32 status, odd;

		if (!bcmring_rx_done(priv, i)) {
			if (!bcmring_rx_resync(priv))
				break;
			continue;
		}

		status = le32_to_cpu(priv->rx_ring[i].sstat);
		len = FIELD_GET(RXF_STATUS_LEN, status);
		priv->rx_head = (i + 1) % RX_RING_SIZE;
		done++;

		/*
		 * Only the length field of the status word is
		 * understood (the vendor driver looks at nothing else).  Report
		 * each bit the first time it differs from the pattern seen
		 * under the vendor firmware, together with the word count the
		 * DMAC wrote back.  Bit 30 is named because it is the only
		 * status bit with no observed value of 1.
		 */
		odd = status ^ (RXF_STATUS_USUAL | (status & RXF_STATUS_LEN) |
				(ALIGN(len, DMA_WORD) & RXF_STATUS_BYTES));
		if (odd & ~priv->rx_status_seen) {
			priv->rx_status_seen |= odd;
			netdev_info(ndev,
				    "Rx status %#010x, length %u, CTL high %#x%s\n",
				    status, len,
				    le32_to_cpu(priv->rx_ring[i].ctl_hi),
				    (odd & BIT(30)) ?
				    " (bit 30 set: never seen under stock firmware)" : "");
		}

		/* The reported length includes the FCS. */
		if (len < ETH_HLEN + ETH_FCS_LEN || len > RX_BUF_SIZE) {
			ndev->stats.rx_length_errors++;
			bcmring_rx_arm(priv, i);
			continue;
		}

		new_skb = bcmring_rx_alloc(priv, &new_dma);
		if (!new_skb) {
			ndev->stats.rx_dropped++;
			bcmring_rx_arm(priv, i);
			continue;
		}

		dma_unmap_single(priv->dev, buf->dma, RX_BUF_SIZE,
				 DMA_FROM_DEVICE);
		skb = buf->skb;
		buf->skb = new_skb;
		buf->dma = new_dma;
		bcmring_rx_arm(priv, i);

		skb_put(skb, len - ETH_FCS_LEN);
		skb->protocol = eth_type_trans(skb, ndev);
		ndev->stats.rx_packets++;
		ndev->stats.rx_bytes += len - ETH_FCS_LEN;
		napi_gro_receive(napi, skb);
	}

	if (done < budget && napi_complete_done(napi, done)) {
		dw_irq_ack(priv, RX_CHAN);
		dw_block_irq(priv, RX_CHAN, true);
		/* A frame may have completed between the last check and the ack. */
		if (bcmring_rx_done(priv, priv->rx_head) &&
		    napi_schedule_prep(napi)) {
			dw_block_irq(priv, RX_CHAN, false);
			__napi_schedule(napi);
		}
	}

	return done;
}

static irqreturn_t bcmring_rx_irq(int irq, void *data)
{
	struct bcmring_enet *priv = data;

	if (!((readl(priv->dmac + DW_STATUS_BLOCK) |
	       readl(priv->dmac + DW_STATUS_ERR)) & BIT(RX_CHAN)))
		return IRQ_NONE;

	dw_block_irq(priv, RX_CHAN, false);
	dw_irq_ack(priv, RX_CHAN);
	napi_schedule(&priv->rx_napi);

	return IRQ_HANDLED;
}

static void bcmring_rx_free(struct bcmring_enet *priv)
{
	unsigned int i;

	for (i = 0; i < RX_RING_SIZE; i++) {
		struct bcmring_rx_buf *buf = &priv->rx_buf[i];

		if (!buf->skb)
			continue;
		dma_unmap_single(priv->dev, buf->dma, RX_BUF_SIZE,
				 DMA_FROM_DEVICE);
		dev_kfree_skb(buf->skb);
		buf->skb = NULL;
	}
}

static int bcmring_rx_init(struct bcmring_enet *priv)
{
	unsigned int i;

	for (i = 0; i < RX_RING_SIZE; i++) {
		struct bcmring_rx_buf *buf = &priv->rx_buf[i];
		struct bcmring_lli *lli = &priv->rx_ring[i];
		unsigned int next = (i + 1) % RX_RING_SIZE;

		buf->skb = bcmring_rx_alloc(priv, &buf->dma);
		if (!buf->skb) {
			bcmring_rx_free(priv);
			return -ENOMEM;
		}

		lli->sar = cpu_to_le32(priv->fifo_phys + RXF_DATA);
		lli->llp = cpu_to_le32(priv->rx_ring_dma + next * sizeof(*lli));
		lli->ctl_lo = cpu_to_le32(RX_CTL_LO);
		bcmring_rx_arm(priv, i);
	}
	priv->rx_head = 0;

	writel(RXF_CFG_VENDOR, priv->fifo + RXF_CFG);
	dw_ch_setup(priv, RX_CHAN, RX_CTL_LO, RX_CFG_HI,
		    priv->fifo_phys + RXF_STATUS, 0);

	return 0;
}

/* ------------------------------------------------------------------------ */
/* Tx                                                                        */
/* ------------------------------------------------------------------------ */

/*
 * Build one LLI chain out of the pending frames and start the channel.  Each
 * frame takes two LLIs: one 64-bit control word to TXF_CTRL, then the frame
 * (rounded out to whole 64-bit words) to TXF_DATA.  The control word tells
 * the bridge which bytes of the first and last word are valid.
 *
 * Like the vendor driver, a chain always starts at LLI 0 and the next chain
 * is only built once the channel has gone idle.
 */
static void bcmring_tx_kick(struct bcmring_enet *priv)
{
	struct bcmring_lli *lli = NULL;
	struct sk_buff *skb;
	unsigned int n = 0;

	lockdep_assert_held(&priv->tx_lock);

	while (n < TX_RING_FRAMES && (skb = __skb_dequeue(&priv->tx_pending))) {
		struct bcmring_tx_buf *buf = &priv->tx_buf[n];
		unsigned int pad = (unsigned long)skb->data & (DMA_WORD - 1);
		unsigned int len = skb->len;	/* includes the placeholder FCS */
		u32 ctrl;

		buf->map_len = ALIGN(pad + len, DMA_WORD);
		buf->dma = dma_map_single(priv->dev, skb->data - pad,
					  buf->map_len, DMA_TO_DEVICE);
		if (dma_mapping_error(priv->dev, buf->dma) ||
		    WARN_ON_ONCE(buf->dma & (DMA_WORD - 1))) {
			if (!dma_mapping_error(priv->dev, buf->dma))
				dma_unmap_single(priv->dev, buf->dma,
						 buf->map_len, DMA_TO_DEVICE);
			priv->ndev->stats.tx_dropped++;
			dev_kfree_skb_any(skb);
			continue;
		}
		buf->skb = skb;
		buf->frame_len = len - ETH_FCS_LEN;

		ctrl = FIELD_PREP(TXF_CTRL_FIRST_OFS, pad) |
		       FIELD_PREP(TXF_CTRL_LAST_OFS, (pad + len - 1) & 7) |
		       TXF_CTRL_BIT16;
		priv->tx_ctrl[n] = cpu_to_le64(ctrl);

		lli = &priv->tx_ring[2 * n];
		lli->sar = cpu_to_le32(priv->tx_ctrl_dma + n * sizeof(__le64));
		lli->dar = cpu_to_le32(priv->fifo_phys + TXF_CTRL);
		lli->llp = cpu_to_le32(priv->tx_ring_dma +
				       (2 * n + 1) * sizeof(*lli));
		lli->ctl_lo = cpu_to_le32(TX_CTL_LO);
		lli->ctl_hi = cpu_to_le32(1);
		lli->sstat = 0;
		lli->dstat = 0;

		lli++;
		lli->sar = cpu_to_le32(buf->dma);
		lli->dar = cpu_to_le32(priv->fifo_phys + TXF_DATA);
		lli->llp = cpu_to_le32(priv->tx_ring_dma +
				       (2 * n + 2) * sizeof(*lli));
		lli->ctl_lo = cpu_to_le32(TX_CTL_LO);
		lli->ctl_hi = cpu_to_le32(buf->map_len / DMA_WORD);
		lli->sstat = 0;
		lli->dstat = 0;

		n++;
	}

	if (!n)
		return;

	/* Terminate the chain and ask for one interrupt at its end. */
	lli->llp = 0;
	lli->ctl_lo = cpu_to_le32(TX_CTL_LO | DW_CTLL_INT_EN);
	dma_wmb();

	priv->tx_inflight = n;
	dw_block_irq(priv, TX_CHAN, true);
	dw_ch_writel(priv, TX_CHAN, DW_LLP, priv->tx_ring_dma);
	writel(DW_CH_SET(TX_CHAN), priv->dmac + DW_CH_EN);
}

static void bcmring_tx_reap(struct bcmring_enet *priv, int budget, bool sent)
{
	unsigned int i;

	lockdep_assert_held(&priv->tx_lock);

	for (i = 0; i < priv->tx_inflight; i++) {
		struct bcmring_tx_buf *buf = &priv->tx_buf[i];
		u32 status = le32_to_cpu(priv->tx_ring[2 * i + 1].dstat);

		/*
		 * The DMAC stores the bridge's Tx status word
		 * in each LLI.  The vendor driver never reads it and its
		 * layout is unknown; report each bit the first time it differs
		 * from the one value seen under the vendor firmware.
		 */
		u32 odd = status ^ TXF_STATUS_USUAL;

		if (sent && (odd & ~priv->tx_status_seen)) {
			priv->tx_status_seen |= odd;
			netdev_info(priv->ndev,
				    "Tx status %#010x after a frame of %u bytes\n",
				    status, buf->frame_len);
		}

		dma_unmap_single(priv->dev, buf->dma, buf->map_len,
				 DMA_TO_DEVICE);
		if (sent) {
			priv->ndev->stats.tx_packets++;
			priv->ndev->stats.tx_bytes += buf->frame_len;
		} else {
			priv->ndev->stats.tx_dropped++;
		}
		napi_consume_skb(buf->skb, budget);
		buf->skb = NULL;
	}
	priv->tx_inflight = 0;
}

static int bcmring_tx_poll(struct napi_struct *napi, int budget)
{
	struct bcmring_enet *priv = container_of(napi, struct bcmring_enet,
						 tx_napi);
	struct net_device *ndev = priv->ndev;
	unsigned long flags;

	spin_lock_irqsave(&priv->tx_lock, flags);
	if (!dw_ch_busy(priv, TX_CHAN)) {
		bcmring_tx_reap(priv, budget, true);
		bcmring_tx_kick(priv);
		if (netif_queue_stopped(ndev) &&
		    skb_queue_len(&priv->tx_pending) < TX_RING_FRAMES / 2)
			netif_wake_queue(ndev);
	} else {
		/* Spurious: the chain is still running; wait for its end. */
		dw_block_irq(priv, TX_CHAN, true);
	}
	spin_unlock_irqrestore(&priv->tx_lock, flags);

	napi_complete(napi);

	return 0;
}

static irqreturn_t bcmring_tx_irq(int irq, void *data)
{
	struct bcmring_enet *priv = data;

	if (!((readl(priv->dmac + DW_STATUS_BLOCK) |
	       readl(priv->dmac + DW_STATUS_ERR)) & BIT(TX_CHAN)))
		return IRQ_NONE;

	dw_block_irq(priv, TX_CHAN, false);
	dw_irq_ack(priv, TX_CHAN);
	napi_schedule(&priv->tx_napi);

	return IRQ_HANDLED;
}

static netdev_tx_t bcmring_start_xmit(struct sk_buff *skb,
				      struct net_device *ndev)
{
	struct bcmring_enet *priv = netdev_priv(ndev);
	unsigned long flags;

	/*
	 * The DSA tagger has already inserted the Broadcom tag and padded the
	 * frame to 64 bytes (the vendor driver: 60 bytes, or 64 for an
	 * 802.1Q-tagged frame, before the tag goes in).  Pad anyway for the
	 * untagged (no switch driver) case.
	 *
	 * The bridge computes no FCS, and the vendor driver hands it four
	 * more bytes than the frame holds: zeros when it had to pad the
	 * frame, otherwise whatever follows the frame in memory.  The switch
	 * is told not to check them (page 0x34 register 0x06 bit 0, set by
	 * the switch driver).  Send zeros.  skb_put_padto() also takes care of
	 * a cloned skb, whose tailroom must not be written.
	 */
	if (skb_put_padto(skb, max_t(unsigned int, skb->len, ETH_ZLEN) +
			  ETH_FCS_LEN)) {
		ndev->stats.tx_dropped++;
		return NETDEV_TX_OK;
	}

	/*
	 * The channel reads whole 64-bit words, i.e., up to seven bytes
	 * before the frame (never sent; keep them inside this skb's buffer)
	 * and up to seven after it (inside the buffer in any case: the shared
	 * info follows the data).
	 */
	if (skb_cow_head(skb, DMA_WORD)) {
		ndev->stats.tx_dropped++;
		dev_kfree_skb_any(skb);
		return NETDEV_TX_OK;
	}

	spin_lock_irqsave(&priv->tx_lock, flags);
	__skb_queue_tail(&priv->tx_pending, skb);
	if (skb_queue_len(&priv->tx_pending) >= TX_RING_FRAMES)
		netif_stop_queue(ndev);
	if (!priv->tx_inflight)
		bcmring_tx_kick(priv);
	spin_unlock_irqrestore(&priv->tx_lock, flags);

	return NETDEV_TX_OK;
}

static void bcmring_tx_init(struct bcmring_enet *priv)
{
	writel(TXF_CFG_VENDOR, priv->fifo + TXF_CFG);
	dw_ch_setup(priv, TX_CHAN, TX_CTL_LO, TX_CFG_HI, 0,
		    priv->fifo_phys + TXF_STATUS);
}

static void bcmring_tx_timeout(struct net_device *ndev, unsigned int txqueue)
{
	struct bcmring_enet *priv = netdev_priv(ndev);

	/* The vendor driver only wakes the queue; run the completion poll. */
	netdev_err(ndev, "Tx timeout, channel %s, %u in flight\n",
		   dw_ch_busy(priv, TX_CHAN) ? "busy" : "idle",
		   priv->tx_inflight);
	ndev->stats.tx_errors++;
	napi_schedule(&priv->tx_napi);
}

/* ------------------------------------------------------------------------ */
/* phylink (fixed internal link to the switch's IMP port)                    */
/* ------------------------------------------------------------------------ */

static void bcmring_mac_config(struct phylink_config *config, unsigned int mode,
			       const struct phylink_link_state *state)
{
}

static void bcmring_mac_link_down(struct phylink_config *config,
				  unsigned int mode, phy_interface_t interface)
{
}

static void bcmring_mac_link_up(struct phylink_config *config,
				struct phy_device *phy, unsigned int mode,
				phy_interface_t interface, int speed, int duplex,
				bool tx_pause, bool rx_pause)
{
	/*
	 * Nothing to program on this side: the FIFO bridge has no speed
	 * setting of its own.  The IMP speed register in the switch wrapper
	 * block is maintained by the switch driver.
	 */
}

static const struct phylink_mac_ops bcmring_phylink_ops = {
	.mac_config = bcmring_mac_config,
	.mac_link_down = bcmring_mac_link_down,
	.mac_link_up = bcmring_mac_link_up,
};

/* ------------------------------------------------------------------------ */
/* netdev                                                                    */
/* ------------------------------------------------------------------------ */

static int bcmring_open(struct net_device *ndev)
{
	struct bcmring_enet *priv = netdev_priv(ndev);
	int ret;

	ret = phylink_of_phy_connect(priv->phylink, priv->dev->of_node, 0);
	if (ret)
		return ret;

	writel(1, priv->dmac + DW_DMA_CFG);
	writel(DW_CH_CLR(RX_CHAN), priv->dmac + DW_CH_EN);
	writel(DW_CH_CLR(TX_CHAN), priv->dmac + DW_CH_EN);

	ret = bcmring_rx_init(priv);
	if (ret)
		goto err_disconnect;
	bcmring_tx_init(priv);

	napi_enable(&priv->rx_napi);
	napi_enable(&priv->tx_napi);

	dw_irq_ack(priv, RX_CHAN);
	dw_block_irq(priv, RX_CHAN, true);
	dw_ch_writel(priv, RX_CHAN, DW_LLP, priv->rx_ring_dma);
	writel(DW_CH_SET(RX_CHAN), priv->dmac + DW_CH_EN);

	phylink_start(priv->phylink);
	netif_start_queue(ndev);

	return 0;

err_disconnect:
	phylink_disconnect_phy(priv->phylink);
	return ret;
}

static int bcmring_stop(struct net_device *ndev)
{
	struct bcmring_enet *priv = netdev_priv(ndev);
	unsigned long flags;

	netif_stop_queue(ndev);
	phylink_stop(priv->phylink);

	dw_block_irq(priv, RX_CHAN, false);
	dw_block_irq(priv, TX_CHAN, false);
	napi_disable(&priv->rx_napi);
	napi_disable(&priv->tx_napi);

	dw_ch_stop(priv, RX_CHAN);
	dw_ch_stop(priv, TX_CHAN);

	spin_lock_irqsave(&priv->tx_lock, flags);
	bcmring_tx_reap(priv, 0, false);
	__skb_queue_purge(&priv->tx_pending);
	spin_unlock_irqrestore(&priv->tx_lock, flags);

	bcmring_rx_free(priv);
	phylink_disconnect_phy(priv->phylink);

	return 0;
}

static const struct net_device_ops bcmring_netdev_ops = {
	.ndo_open = bcmring_open,
	.ndo_stop = bcmring_stop,
	.ndo_start_xmit = bcmring_start_xmit,
	.ndo_tx_timeout = bcmring_tx_timeout,
	.ndo_set_mac_address = eth_mac_addr,
	.ndo_validate_addr = eth_validate_addr,
};

/* ------------------------------------------------------------------------ */
/* Platform driver                                                           */
/* ------------------------------------------------------------------------ */

static void bcmring_phylink_destroy(void *data)
{
	phylink_destroy(data);
}

static int bcmring_probe(struct platform_device *pdev)
{
	struct device *dev = &pdev->dev;
	struct device_node *np = dev->of_node;
	struct clk_bulk_data *clks;
	struct bcmring_enet *priv;
	struct net_device *ndev;
	phy_interface_t iface;
	struct resource *res;
	int ret;

	ndev = devm_alloc_etherdev(dev, sizeof(*priv));
	if (!ndev)
		return -ENOMEM;
	SET_NETDEV_DEV(ndev, dev);
	platform_set_drvdata(pdev, ndev);

	priv = netdev_priv(ndev);
	priv->dev = dev;
	priv->ndev = ndev;
	spin_lock_init(&priv->tx_lock);
	__skb_queue_head_init(&priv->tx_pending);

	res = platform_get_resource_byname(pdev, IORESOURCE_MEM, "fifo");
	priv->fifo = devm_ioremap_resource(dev, res);
	if (IS_ERR(priv->fifo))
		return PTR_ERR(priv->fifo);
	priv->fifo_phys = res->start;

	/*
	 * Not requested exclusively: the block is shared with the other six
	 * channels of the controller (see the top of the file).
	 */
	res = platform_get_resource_byname(pdev, IORESOURCE_MEM, "dmac");
	if (!res)
		return dev_err_probe(dev, -EINVAL, "missing dmac resource\n");
	priv->dmac = devm_ioremap(dev, res->start, resource_size(res));
	if (!priv->dmac)
		return -ENOMEM;

	/*
	 * ESW core clock, its bus-interface gate, and the bus-interface gate
	 * of the DMA controller whose channels are used.  The switch driver
	 * holds the first two as well and releases the block's resets.
	 */
	ret = devm_clk_bulk_get_all_enabled(dev, &clks);
	if (ret < 0)
		return dev_err_probe(dev, ret, "failed to enable clocks\n");

	priv->rx_irq = platform_get_irq_byname(pdev, "rx");
	if (priv->rx_irq < 0)
		return priv->rx_irq;
	priv->tx_irq = platform_get_irq_byname(pdev, "tx");
	if (priv->tx_irq < 0)
		return priv->tx_irq;

	ret = dma_set_mask_and_coherent(dev, DMA_BIT_MASK(32));
	if (ret)
		return ret;

	priv->rx_ring = dmam_alloc_coherent(dev,
					    RX_RING_SIZE * sizeof(*priv->rx_ring),
					    &priv->rx_ring_dma, GFP_KERNEL);
	priv->tx_ring = dmam_alloc_coherent(dev, 2 * TX_RING_FRAMES *
					    sizeof(*priv->tx_ring),
					    &priv->tx_ring_dma, GFP_KERNEL);
	priv->tx_ctrl = dmam_alloc_coherent(dev,
					    TX_RING_FRAMES * sizeof(*priv->tx_ctrl),
					    &priv->tx_ctrl_dma, GFP_KERNEL);
	if (!priv->rx_ring || !priv->tx_ring || !priv->tx_ctrl)
		return -ENOMEM;

	/* Take over the controller; everything stays quiet until open. */
	bcmring_dmac_quiesce(priv);
	ret = devm_add_action_or_reset(dev, bcmring_dmac_off, priv);
	if (ret)
		return ret;

	ret = devm_request_irq(dev, priv->rx_irq, bcmring_rx_irq, 0,
			       DRV_NAME "-rx", priv);
	if (ret)
		return ret;
	ret = devm_request_irq(dev, priv->tx_irq, bcmring_tx_irq, 0,
			       DRV_NAME "-tx", priv);
	if (ret)
		return ret;

	/*
	 * There is no hardware address source.  The factory address exists
	 * only as text (ENETCFG_MAC=) in the file etc/enet.cfg of the JFFS2
	 * file system in the "nvdata" flash partition; the vendor's boot
	 * scripts read it and set it with ifconfig.  That cannot be an NVMEM
	 * cell, so on these phones it stays userspace's job (or that of a
	 * boot loader that fills local-mac-address, hence the ethernet0
	 * alias).  Honor the devicetree or NVMEM if they do provide an
	 * address, else start with a random one.  DSA user ports copy the
	 * conduit's address when they are created, so userspace has to set
	 * the address on them as well.
	 */
	ret = platform_get_ethdev_address(dev, ndev);
	if (ret == -EPROBE_DEFER)
		return ret;
	if (ret)
		eth_hw_addr_random(ndev);

	ret = of_get_phy_mode(np, &iface);
	if (ret)
		iface = PHY_INTERFACE_MODE_INTERNAL;

	priv->phylink_config.dev = &ndev->dev;
	priv->phylink_config.type = PHYLINK_NETDEV;
	priv->phylink_config.mac_capabilities = MAC_10FD | MAC_100FD |
						MAC_1000FD;
	__set_bit(PHY_INTERFACE_MODE_INTERNAL,
		  priv->phylink_config.supported_interfaces);
	__set_bit(PHY_INTERFACE_MODE_GMII,
		  priv->phylink_config.supported_interfaces);

	priv->phylink = phylink_create(&priv->phylink_config,
				       of_fwnode_handle(np), iface,
				       &bcmring_phylink_ops);
	if (IS_ERR(priv->phylink))
		return PTR_ERR(priv->phylink);
	ret = devm_add_action_or_reset(dev, bcmring_phylink_destroy,
				       priv->phylink);
	if (ret)
		return ret;

	ndev->netdev_ops = &bcmring_netdev_ops;
	ndev->watchdog_timeo = 5 * HZ;
	ndev->min_mtu = ETH_MIN_MTU;
	ndev->max_mtu = BCMRING_MAX_MTU;
	/* Slack before the frame is read (not sent) by the 64-bit Tx channel. */
	ndev->needed_headroom = DMA_WORD;
	ndev->needed_tailroom = ETH_FCS_LEN;

	netif_napi_add(ndev, &priv->rx_napi, bcmring_rx_poll);
	netif_napi_add_tx(ndev, &priv->tx_napi, bcmring_tx_poll);

	return devm_register_netdev(dev, ndev);
}

static const struct of_device_id bcmring_enet_of_match[] = {
	{ .compatible = "brcm,bcmring-enet" },
	{ }
};
MODULE_DEVICE_TABLE(of, bcmring_enet_of_match);

static struct platform_driver bcmring_enet_driver = {
	.probe = bcmring_probe,
	.driver = {
		.name = DRV_NAME,
		.of_match_table = bcmring_enet_of_match,
	},
};
module_platform_driver(bcmring_enet_driver);

MODULE_DESCRIPTION("Broadcom BCMRING embedded-switch CPU port driver");
MODULE_LICENSE("GPL");
