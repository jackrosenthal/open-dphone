// SPDX-License-Identifier: GPL-2.0-only
/*
 * Broadcom BCMRING "UMI" raw NAND controller (vendor name: bcm_umi-nand).
 *
 * FIRST DRAFT.  Compile-tested only; never run on hardware.
 *
 * The register definitions, the controller initialisation, and the hardware
 * BCH page read/write/correct sequences are adapted from the GPL driver that
 * was in mainline Linux up to v3.5 (drivers/mtd/nand/bcm_umi_nand.c,
 * bcm_umi_bch.c, nand_bcm_umi.[ch] and arch/arm/mach-bcmring/include/mach/
 * reg_umi.h, reg_nand.h; Copyright 2004 - 2009 Broadcom Corporation), and
 * were checked against the vendor 3.4.74 kernel of the Digium D60
 * (bcm_umi_nand_inithw, bcm_umi_bch_read_page_hwecc,
 * bcm_umi_bch_write_page_hwecc, nand_bcm_umi_bch_correct_page).  The
 * exec_op plumbing and the software BCH path are new.  See ../NOTES.md.
 *
 * On-flash format (verified against a full raw dump of a D60, 2048 + 64):
 *   - four 512-byte ECC steps per 2 KiB page, 16 OOB bytes per step;
 *   - OOB byte 0: bad-block marker; bytes 1..2: free (JFFS2 cleanmarker);
 *   - OOB bytes 16 * step + 3 .. 16 * step + 15: 13 ECC bytes;
 *   - ECC = BCH over GF(2^13), primitive polynomial 0x201b, t = 8, computed
 *     over the 512 data bytes only, data bits taken MSB first, remainder
 *     stored big-endian.  This is exactly what lib/bch.c computes with
 *     swap_bits = false and no ECC mask, so the software path below reads
 *     and writes pages the vendor boot loaders and kernel accept.
 *
 * ECC is done in software by default.  Loading with hw_bch=1 uses the
 * controller's BCH engine with the vendor sequence instead.
 */

#include <linux/bch.h>
#include <linux/bits.h>
#include <linux/clk.h>
#include <linux/delay.h>
#include <linux/io.h>
#include <linux/iopoll.h>
#include <linux/module.h>
#include <linux/mtd/mtd.h>
#include <linux/mtd/rawnand.h>
#include <linux/of.h>
#include <linux/platform_device.h>
#include <linux/slab.h>

/* UMI controller registers (first reg entry) */
#define UMI_MMD_ICR			0x0c
#define   UMI_MMD_ICR_FLASH_WP		BIT(15)	/* set = not write protected */
#define UMI_NAND_TCR			0x18
#define   UMI_NAND_TCR_CS_SWCTRL	BIT(31)
#define   UMI_NAND_TCR_WORD16		BIT(30)
#define   UMI_NAND_TCR_TIMING_MASK	GENMASK(18, 0)
/*
 * Timing value the vendor kernel programs (tBTA, tWP, tWR, tAS, tOE,
 * tRC/tLC fields).  TODO: derive from the SDR timings and the bus clock in
 * a ->setup_interface() hook; the field encodings are not recovered.
 */
#define   UMI_NAND_TCR_TIMING_VENDOR	0x00052267
#define UMI_NAND_RCSR			0x1c
#define   UMI_NAND_RCSR_RDY		BIT(1)
#define   UMI_NAND_RCSR_CS_ASSERTED	BIT(0)
#define UMI_NAND_ECC_CSR		0x20	/* Hamming engine; unused */
#define UMI_BCH_N			0x40
#define UMI_BCH_K			0x44
#define UMI_BCH_T			0x48
#define UMI_BCH_CTRL_STATUS		0x4c
#define   UMI_BCH_NB_CORR_ERROR		GENMASK(23, 20)
#define   UMI_BCH_UNCORR_ERR		BIT(19)
#define   UMI_BCH_CORR_ERR		BIT(18)
#define   UMI_BCH_RD_ECC_VALID		BIT(17)
#define   UMI_BCH_WR_ECC_VALID		BIT(16)
#define   UMI_BCH_PAUSE_ECC_DEC		BIT(4)
#define   UMI_BCH_ECC_RD_EN		BIT(1)
#define   UMI_BCH_ECC_WR_EN		BIT(0)
#define UMI_BCH_WR_ECC(n)		(0x50 + 4 * (n))
#define UMI_BCH_RD_ERR_LOC(n)		(0x64 + 4 * ((n) / 2))
#define   UMI_BCH_ERR_LOC_MASK		GENMASK(12, 0)

/* NAND aperture (second reg entry) */
#define NAND_CMD_REG			0x000
#define NAND_ADDR_REG			0x004
#define NAND_DATA_REG			0x008
#define NAND_DATA_DMA_REG		0x400	/* 32-bit DMA port; unused */

#define BCMRING_ECC_STEP		512
#define BCMRING_ECC_BYTES		13
#define BCMRING_ECC_STRENGTH		8
#define BCMRING_ECC_M			13
#define BCMRING_ECC_PRIM_POLY		0x201b
#define BCMRING_OOB_PER_STEP		16
#define BCMRING_OOB_ECC_OFFSET		3

static bool hw_bch;
module_param(hw_bch, bool, 0444);
MODULE_PARM_DESC(hw_bch, "Use the controller's BCH engine instead of software BCH");

struct bcmring_nand {
	struct device *dev;
	void __iomem *umi;
	void __iomem *io;
	struct nand_controller controller;
	struct nand_chip chip;
	struct bch_control *bch;
	unsigned int errloc[BCMRING_ECC_STRENGTH];
};

static struct bcmring_nand *to_bcmring_nand(struct nand_chip *chip)
{
	return container_of(chip, struct bcmring_nand, chip);
}

/* ---- OOB layout --------------------------------------------------------- */

static int bcmring_ooblayout_ecc(struct mtd_info *mtd, int section,
				 struct mtd_oob_region *oobregion)
{
	struct nand_chip *chip = mtd_to_nand(mtd);

	if (section >= chip->ecc.steps)
		return -ERANGE;

	oobregion->offset = section * BCMRING_OOB_PER_STEP +
			    BCMRING_OOB_ECC_OFFSET;
	oobregion->length = BCMRING_ECC_BYTES;
	return 0;
}

static int bcmring_ooblayout_free(struct mtd_info *mtd, int section,
				  struct mtd_oob_region *oobregion)
{
	/*
	 * Same as the vendor layout: only bytes 1 and 2 of the page OOB.
	 * (Bytes 0..2 of steps 1..3 are physically unused as well, but the
	 * vendor never exposed them.)
	 */
	if (section)
		return -ERANGE;

	oobregion->offset = 1;
	oobregion->length = 2;
	return 0;
}

static const struct mtd_ooblayout_ops bcmring_ooblayout_ops = {
	.ecc = bcmring_ooblayout_ecc,
	.free = bcmring_ooblayout_free,
};

/* ---- low-level bus access ----------------------------------------------- */

static int bcmring_nand_waitrdy(struct bcmring_nand *nfc,
				unsigned int timeout_ms)
{
	u32 val;

	/* tWB: R/B# may take up to 100 ns to fall after the command. */
	udelay(1);
	return readl_poll_timeout(nfc->umi + UMI_NAND_RCSR, val,
				  val & UMI_NAND_RCSR_RDY, 10,
				  timeout_ms * 1000);
}

static int bcmring_nand_exec_instr(struct bcmring_nand *nfc,
				   const struct nand_op_instr *instr)
{
	unsigned int i;
	int ret = 0;

	switch (instr->type) {
	case NAND_OP_CMD_INSTR:
		writeb(instr->ctx.cmd.opcode, nfc->io + NAND_CMD_REG);
		break;
	case NAND_OP_ADDR_INSTR:
		for (i = 0; i < instr->ctx.addr.naddrs; i++)
			writeb(instr->ctx.addr.addrs[i],
			       nfc->io + NAND_ADDR_REG);
		break;
	case NAND_OP_DATA_IN_INSTR:
		ioread8_rep(nfc->io + NAND_DATA_REG, instr->ctx.data.buf.in,
			    instr->ctx.data.len);
		break;
	case NAND_OP_DATA_OUT_INSTR:
		iowrite8_rep(nfc->io + NAND_DATA_REG, instr->ctx.data.buf.out,
			     instr->ctx.data.len);
		break;
	case NAND_OP_WAITRDY_INSTR:
		ret = bcmring_nand_waitrdy(nfc, instr->ctx.waitrdy.timeout_ms);
		break;
	}

	if (instr->delay_ns)
		ndelay(instr->delay_ns);

	return ret;
}

static int bcmring_nand_exec_op(struct nand_chip *chip,
				const struct nand_operation *op,
				bool check_only)
{
	struct bcmring_nand *nfc = to_bcmring_nand(chip);
	unsigned int i;
	int ret;

	if (check_only)
		return 0;

	/*
	 * Chip select is under software control (NAND_TCR.CS_SWCTRL) and the
	 * vendor code asserts it once at init and never releases it; there is
	 * a single chip select, so do the same.
	 */
	for (i = 0; i < op->ninstrs; i++) {
		ret = bcmring_nand_exec_instr(nfc, &op->instrs[i]);
		if (ret)
			return ret;
	}

	return 0;
}

/* ---- software BCH -------------------------------------------------------- */

static u8 *bcmring_oob_ecc(struct nand_chip *chip, int step)
{
	return chip->oob_poi + step * BCMRING_OOB_PER_STEP +
	       BCMRING_OOB_ECC_OFFSET;
}

static int bcmring_nand_read_page_sw(struct nand_chip *chip, u8 *buf,
				     int oob_required, int page)
{
	struct bcmring_nand *nfc = to_bcmring_nand(chip);
	struct mtd_info *mtd = nand_to_mtd(chip);
	unsigned int max_bitflips = 0;
	u8 calc[BCMRING_ECC_BYTES];
	int step, i, ret, stat;

	ret = nand_read_page_op(chip, page, 0, buf, mtd->writesize);
	if (ret)
		return ret;
	ret = nand_read_data_op(chip, chip->oob_poi, mtd->oobsize, false,
				false);
	if (ret)
		return ret;

	for (step = 0; step < chip->ecc.steps; step++) {
		u8 *data = buf + step * chip->ecc.size;
		u8 *code = bcmring_oob_ecc(chip, step);

		memset(calc, 0, sizeof(calc));
		bch_encode(nfc->bch, data, chip->ecc.size, calc);
		if (!memcmp(calc, code, sizeof(calc)))
			continue;

		/*
		 * An erased step carries no valid ECC (the ECC of 512 0xff
		 * bytes is not all-0xff in this format).  The vendor driver
		 * treats "uncorrectable and all ECC bytes 0xff" as erased
		 * and forces the data to 0xff; the core helper does the same
		 * with a bitflip budget.  It is tried before the decoder so
		 * that a nearly blank step can never be "corrected" towards
		 * some unrelated codeword.
		 */
		stat = nand_check_erased_ecc_chunk(data, chip->ecc.size, code,
						   BCMRING_ECC_BYTES, NULL, 0,
						   chip->ecc.strength);
		if (stat < 0) {
			stat = bch_decode(nfc->bch, NULL, chip->ecc.size, code,
					  calc, NULL, nfc->errloc);
			for (i = 0; i < stat; i++) {
				unsigned int loc = nfc->errloc[i];

				/* locations past the data are in the ECC */
				if (loc < chip->ecc.size * 8)
					data[loc >> 3] ^= BIT(loc & 7);
			}
		}

		if (stat < 0) {
			mtd->ecc_stats.failed++;
		} else {
			mtd->ecc_stats.corrected += stat;
			max_bitflips = max_t(unsigned int, max_bitflips, stat);
		}
	}

	return max_bitflips;
}

static int bcmring_nand_write_page_sw(struct nand_chip *chip, const u8 *buf,
				      int oob_required, int page)
{
	struct bcmring_nand *nfc = to_bcmring_nand(chip);
	struct mtd_info *mtd = nand_to_mtd(chip);
	int step, ret;

	for (step = 0; step < chip->ecc.steps; step++) {
		u8 *code = bcmring_oob_ecc(chip, step);

		memset(code, 0, BCMRING_ECC_BYTES);
		bch_encode(nfc->bch, buf + step * chip->ecc.size,
			   chip->ecc.size, code);
	}

	ret = nand_prog_page_begin_op(chip, page, 0, buf, mtd->writesize);
	if (ret)
		return ret;
	ret = nand_write_data_op(chip, chip->oob_poi, mtd->oobsize, false);
	if (ret)
		return ret;

	return nand_prog_page_end_op(chip);
}

/* ---- hardware BCH (vendor sequence) -------------------------------------- */

static void bcmring_bch_flip_bit(u8 *data, u32 loc)
{
	unsigned int bit = loc & 0x7;
	unsigned int byte_in_word = 3 - ((loc >> 3) & 0x3);
	unsigned int word = (loc >> 5) & 0xff;

	data[word * 4 + byte_in_word] ^= BIT(bit);
}

static int bcmring_bch_correct(struct bcmring_nand *nfc, u8 *data,
			       unsigned int len, const u8 *code)
{
	struct nand_chip *chip = &nfc->chip;
	unsigned int i, nerr;
	u32 status, loc;
	int ret;

	ret = readl_poll_timeout(nfc->umi + UMI_BCH_CTRL_STATUS, status,
				 status & UMI_BCH_RD_ECC_VALID, 0, 100000);
	if (ret)
		return ret;

	if (status & UMI_BCH_UNCORR_ERR)
		return nand_check_erased_ecc_chunk(data, len, (void *)code,
						   BCMRING_ECC_BYTES, NULL, 0,
						   chip->ecc.strength);

	if (!(status & UMI_BCH_CORR_ERR))
		return 0;

	nerr = (status & UMI_BCH_NB_CORR_ERROR) >> 20;
	for (i = 0; i < nerr; i++) {
		loc = readl(nfc->umi + UMI_BCH_RD_ERR_LOC(i));
		loc = (loc >> ((i % 2) * 16)) & UMI_BCH_ERR_LOC_MASK;
		/*
		 * TODO: confirm on hardware that locations inside the ECC
		 * bytes are never reported with an index below 512 * 8.
		 */
		if ((loc >> 5) * 4 < len)
			bcmring_bch_flip_bit(data, loc);
	}

	return nerr;
}

static int bcmring_nand_read_page_hw(struct nand_chip *chip, u8 *buf,
				     int oob_required, int page)
{
	struct bcmring_nand *nfc = to_bcmring_nand(chip);
	struct mtd_info *mtd = nand_to_mtd(chip);
	unsigned int max_bitflips = 0;
	int step, ret, stat;

	ret = nand_read_page_op(chip, page, 0, NULL, 0);
	if (ret)
		return ret;

	for (step = 0; step < chip->ecc.steps; step++) {
		u8 *data = buf + step * chip->ecc.size;
		u8 *oob = chip->oob_poi + step * BCMRING_OOB_PER_STEP;

		if (step) {
			ret = nand_change_read_column_op(chip,
							 step * chip->ecc.size,
							 NULL, 0, false);
			if (ret)
				return ret;
		}

		/* clear the valid flag, then enable the decoder */
		writel(UMI_BCH_RD_ECC_VALID, nfc->umi + UMI_BCH_CTRL_STATUS);
		writel(UMI_BCH_ECC_RD_EN, nfc->umi + UMI_BCH_CTRL_STATUS);

		ret = nand_read_data_op(chip, data, chip->ecc.size, false,
					false);
		if (ret)
			goto out_disable;

		/* pause while seeking to, and skipping, the non-ECC OOB */
		writel(UMI_BCH_ECC_RD_EN | UMI_BCH_PAUSE_ECC_DEC,
		       nfc->umi + UMI_BCH_CTRL_STATUS);

		ret = nand_change_read_column_op(chip, mtd->writesize +
						 step * BCMRING_OOB_PER_STEP,
						 oob, BCMRING_OOB_ECC_OFFSET,
						 false);
		if (ret)
			goto out_disable;

		writel(UMI_BCH_ECC_RD_EN, nfc->umi + UMI_BCH_CTRL_STATUS);

		ret = nand_read_data_op(chip, oob + BCMRING_OOB_ECC_OFFSET,
					BCMRING_ECC_BYTES, false, false);
		if (ret)
			goto out_disable;

		stat = bcmring_bch_correct(nfc, data, chip->ecc.size,
					   oob + BCMRING_OOB_ECC_OFFSET);
		if (stat < 0) {
			mtd->ecc_stats.failed++;
		} else {
			mtd->ecc_stats.corrected += stat;
			max_bitflips = max_t(unsigned int, max_bitflips, stat);
		}
	}

	ret = max_bitflips;

out_disable:
	writel(UMI_BCH_RD_ECC_VALID | UMI_BCH_WR_ECC_VALID,
	       nfc->umi + UMI_BCH_CTRL_STATUS);
	return ret;
}

static int bcmring_nand_write_page_hw(struct nand_chip *chip, const u8 *buf,
				      int oob_required, int page)
{
	struct bcmring_nand *nfc = to_bcmring_nand(chip);
	struct mtd_info *mtd = nand_to_mtd(chip);
	int step, i, ret;
	u32 status, w;

	ret = nand_prog_page_begin_op(chip, page, 0, NULL, 0);
	if (ret)
		return ret;

	for (step = 0; step < chip->ecc.steps; step++) {
		u8 *code = bcmring_oob_ecc(chip, step);

		writel(UMI_BCH_WR_ECC_VALID, nfc->umi + UMI_BCH_CTRL_STATUS);
		writel(UMI_BCH_ECC_WR_EN, nfc->umi + UMI_BCH_CTRL_STATUS);

		ret = nand_write_data_op(chip, buf + step * chip->ecc.size,
					 chip->ecc.size, false);
		if (ret)
			goto out_disable;

		ret = readl_poll_timeout(nfc->umi + UMI_BCH_CTRL_STATUS,
					 status, status & UMI_BCH_WR_ECC_VALID,
					 0, 100000);
		if (ret)
			goto out_disable;

		/*
		 * 104 parity bits, most significant byte first:
		 * WR_ECC_3[7:0], then WR_ECC_2, WR_ECC_1, WR_ECC_0 each
		 * from bit 31 down.
		 */
		code[0] = readl(nfc->umi + UMI_BCH_WR_ECC(3)) & 0xff;
		for (i = 0; i < 3; i++) {
			w = readl(nfc->umi + UMI_BCH_WR_ECC(2 - i));
			code[1 + 4 * i] = w >> 24;
			code[2 + 4 * i] = w >> 16;
			code[3 + 4 * i] = w >> 8;
			code[4 + 4 * i] = w;
		}
	}

	ret = nand_write_data_op(chip, chip->oob_poi, mtd->oobsize, false);

out_disable:
	writel(UMI_BCH_RD_ECC_VALID | UMI_BCH_WR_ECC_VALID,
	       nfc->umi + UMI_BCH_CTRL_STATUS);
	if (ret)
		return ret;

	return nand_prog_page_end_op(chip);
}

/* ---- controller setup ----------------------------------------------------- */

static int bcmring_nand_attach_chip(struct nand_chip *chip)
{
	struct bcmring_nand *nfc = to_bcmring_nand(chip);
	struct mtd_info *mtd = nand_to_mtd(chip);

	/*
	 * Only the large-page layout (16 OOB bytes per 512-byte step) is
	 * implemented.  The vendor driver also has a 512-byte-page layout
	 * with a different ECC placement; no supported phone uses it.
	 */
	if (mtd->writesize < 2048 ||
	    mtd->oobsize < mtd->writesize / BCMRING_ECC_STEP *
			   BCMRING_OOB_PER_STEP) {
		dev_err(nfc->dev, "unsupported NAND geometry %u+%u\n",
			mtd->writesize, mtd->oobsize);
		return -EINVAL;
	}

	if (chip->options & NAND_BUSWIDTH_16) {
		dev_err(nfc->dev, "16-bit NAND is not supported\n");
		return -EINVAL;
	}

	chip->ecc.engine_type = NAND_ECC_ENGINE_TYPE_ON_HOST;
	chip->ecc.size = BCMRING_ECC_STEP;
	chip->ecc.bytes = BCMRING_ECC_BYTES;
	chip->ecc.strength = BCMRING_ECC_STRENGTH;
	chip->ecc.read_page_raw = nand_read_page_raw;
	chip->ecc.write_page_raw = nand_write_page_raw;
	chip->ecc.read_oob = nand_read_oob_std;
	chip->ecc.write_oob = nand_write_oob_std;

	if (hw_bch) {
		chip->ecc.read_page = bcmring_nand_read_page_hw;
		chip->ecc.write_page = bcmring_nand_write_page_hw;
	} else {
		chip->ecc.read_page = bcmring_nand_read_page_sw;
		chip->ecc.write_page = bcmring_nand_write_page_sw;
	}

	mtd_set_ooblayout(mtd, &bcmring_ooblayout_ops);

	return 0;
}

static const struct nand_controller_ops bcmring_nand_controller_ops = {
	.attach_chip = bcmring_nand_attach_chip,
	.exec_op = bcmring_nand_exec_op,
};

static int bcmring_nand_init_hw(struct bcmring_nand *nfc)
{
	u32 val;
	int ret;

	/* Same order as the vendor bcm_umi_nand_inithw(). */
	val = readl(nfc->umi + UMI_NAND_TCR);
	val &= ~UMI_NAND_TCR_TIMING_MASK;
	val |= UMI_NAND_TCR_TIMING_VENDOR;
	writel(val, nfc->umi + UMI_NAND_TCR);

	val |= UMI_NAND_TCR_CS_SWCTRL;
	writel(val, nfc->umi + UMI_NAND_TCR);

	writel(readl(nfc->umi + UMI_NAND_RCSR) | UMI_NAND_RCSR_CS_ASSERTED,
	       nfc->umi + UMI_NAND_RCSR);

	val &= ~UMI_NAND_TCR_WORD16;
	writel(val, nfc->umi + UMI_NAND_TCR);

	/* release write protect */
	writel(readl(nfc->umi + UMI_MMD_ICR) | UMI_MMD_ICR_FLASH_WP,
	       nfc->umi + UMI_MMD_ICR);

	writeb(NAND_CMD_RESET, nfc->io + NAND_CMD_REG);
	ret = bcmring_nand_waitrdy(nfc, 250);
	if (ret) {
		dev_err(nfc->dev, "NAND did not become ready after reset\n");
		return ret;
	}

	/*
	 * BCH engine idle, then n/k/t for 512 data + 13 ECC bytes:
	 * n = 4200, k = 4096, t = 8.
	 */
	writel(UMI_BCH_RD_ECC_VALID | UMI_BCH_WR_ECC_VALID,
	       nfc->umi + UMI_BCH_CTRL_STATUS);
	writel((BCMRING_ECC_STEP + BCMRING_ECC_BYTES) * 8,
	       nfc->umi + UMI_BCH_N);
	writel(BCMRING_ECC_STRENGTH, nfc->umi + UMI_BCH_T);
	writel(BCMRING_ECC_STEP * 8, nfc->umi + UMI_BCH_K);

	return 0;
}

static void bcmring_nand_free_bch(void *data)
{
	bch_free(data);
}

static int bcmring_nand_probe(struct platform_device *pdev)
{
	struct device *dev = &pdev->dev;
	struct device_node *np;
	struct bcmring_nand *nfc;
	struct nand_chip *chip;
	struct mtd_info *mtd;
	struct clk *clk;
	int ret;

	nfc = devm_kzalloc(dev, sizeof(*nfc), GFP_KERNEL);
	if (!nfc)
		return -ENOMEM;
	nfc->dev = dev;

	nfc->umi = devm_platform_ioremap_resource(pdev, 0);
	if (IS_ERR(nfc->umi))
		return PTR_ERR(nfc->umi);
	nfc->io = devm_platform_ioremap_resource(pdev, 1);
	if (IS_ERR(nfc->io))
		return PTR_ERR(nfc->io);

	/*
	 * The vendor driver takes no clock.  The bus-interface gate that
	 * feeds this block is assumed to be "NVI" (not verified); it is
	 * optional here.
	 */
	clk = devm_clk_get_optional_enabled(dev, NULL);
	if (IS_ERR(clk))
		return dev_err_probe(dev, PTR_ERR(clk), "clock\n");

	nfc->bch = bch_init(BCMRING_ECC_M, BCMRING_ECC_STRENGTH,
			    BCMRING_ECC_PRIM_POLY, false);
	if (!nfc->bch)
		return -ENOMEM;
	ret = devm_add_action_or_reset(dev, bcmring_nand_free_bch, nfc->bch);
	if (ret)
		return ret;
	if (nfc->bch->ecc_bytes != BCMRING_ECC_BYTES)
		return -EINVAL;

	ret = bcmring_nand_init_hw(nfc);
	if (ret)
		return ret;

	nand_controller_init(&nfc->controller);
	nfc->controller.ops = &bcmring_nand_controller_ops;

	chip = &nfc->chip;
	chip->controller = &nfc->controller;
	mtd = nand_to_mtd(chip);
	mtd->dev.parent = dev;

	/* first "nand@0" style child if present, else the controller node */
	np = of_get_next_available_child(dev->of_node, NULL);
	nand_set_flash_node(chip, np ? np : dev->of_node);
	of_node_put(np);

	/*
	 * boot2 passes "mtdparts=bcm_umi-nand:..." on the command line; keep
	 * the vendor MTD name so that cmdlinepart applies it.
	 */
	if (!mtd->name)
		mtd->name = "bcm_umi-nand";

	/*
	 * Bad-block handling as in the vendor driver: factory markers only
	 * (OOB byte 0), in-RAM table built by scanning at attach; no
	 * on-flash BBT.
	 */
	ret = nand_scan(chip, 1);
	if (ret)
		return ret;

	ret = mtd_device_register(mtd, NULL, 0);
	if (ret) {
		nand_cleanup(chip);
		return ret;
	}

	platform_set_drvdata(pdev, nfc);
	return 0;
}

static void bcmring_nand_remove(struct platform_device *pdev)
{
	struct bcmring_nand *nfc = platform_get_drvdata(pdev);
	struct nand_chip *chip = &nfc->chip;

	WARN_ON(mtd_device_unregister(nand_to_mtd(chip)));
	nand_cleanup(chip);
}

static const struct of_device_id bcmring_nand_of_match[] = {
	{ .compatible = "brcm,bcmring-nand" },
	{ }
};
MODULE_DEVICE_TABLE(of, bcmring_nand_of_match);

static struct platform_driver bcmring_nand_driver = {
	.probe = bcmring_nand_probe,
	.remove = bcmring_nand_remove,
	.driver = {
		.name = "bcmring-nand",
		.of_match_table = bcmring_nand_of_match,
	},
};
module_platform_driver(bcmring_nand_driver);

MODULE_DESCRIPTION("Broadcom BCMRING UMI NAND controller driver");
MODULE_LICENSE("GPL");
