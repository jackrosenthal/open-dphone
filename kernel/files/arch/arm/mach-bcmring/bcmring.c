// SPDX-License-Identifier: GPL-2.0-only
/*
 * Broadcom BCMRING (BCM11107 family) devicetree machine support.
 *
 * FIRST DRAFT.  Compile-tested only; never run on hardware.
 *
 * Two things cannot be expressed with existing drivers and devicetree:
 *
 *  1. "Unsecure" setup.  The SoC is an ARM1176JZ-S with TrustZone glue.
 *     boot2 leaves a number of peripherals (chip control, UARTs, watchdog,
 *     interrupt controller, ...) assigned to the secure world, in which case
 *     their interrupts are delivered through the secure interrupt
 *     controller (SINTC) as FIQs.  Every vendor kernel first enables the SPU
 *     bus-interface clock and then writes two "set unsecure" masks, before
 *     registering any device (bcmring_amba_init: D60 0xc03a9f1c, D40
 *     0xc000c490).  The values below are the vendor's, plus the Ethernet
 *     switch bit that the vendor Ethernet code sets later.
 *
 *  2. Restart.  The vendor chipcHw_reset() (D60 0xc001cd38) copies a small
 *     routine into the on-chip ARAM and runs it from there, because the
 *     routine switches the ARM clock to the crystal before it asserts the
 *     chip reset and must not be executing from DDR while it does so.  The
 *     MMU stays on (the vendor routine uses virtual addresses).
 *     The SP805 watchdog restart handler (with the 2 KiB decode patch) is
 *     the simpler alternative and is registered at a lower priority.
 *
 * The reset sequence follows the vendor binary; the register names come from
 * the GPL mach-bcmring code that was in mainline Linux up to v3.5
 * (arch/arm/mach-bcmring/csp/chipc/chipcHw_reset.c, secHw_inline.h).
 */

#include <linux/init.h>
#include <linux/io.h>
#include <linux/irqchip.h>
#include <linux/kexec.h>
#include <linux/of.h>
#include <linux/of_address.h>
#include <linux/reboot.h>
#include <linux/sizes.h>
#include <linux/syscore_ops.h>

#include <asm/cacheflush.h>
#include <asm/fncpy.h>
#include <asm/mach/arch.h>

/* chip control */
#define CHIPC_ARM_CLK		0x08
#define CHIPC_UART_CLK		0x18
#define CHIPC_SPI_CLK		0x24
#define CHIPC_BUSINTF_CLK	0x5c
#define   CHIPC_BUSINTF_DMAC0		BIT(5)
#define   CHIPC_BUSINTF_DMAC1		BIT(6)
#define CHIPC_SOFT_RESET1	0x80
#define CHIPC_SOFT_RESET2	0x84
#define   CHIPC_CLK_BYPASS		BIT(17)
#define   CHIPC_BUSINTF_SPU		BIT(19)
#define   CHIPC_BUSINTF_VRAM		BIT(13)
#define   CHIPC_SOFT_RESET_CHIP_SOFT	BIT(0)
#define   CHIPC_SOFT_RESET_LCD		BIT(22)
#define   CHIPC_SOFT_RESET_CHIP_WARM	BIT(1)

/* "TZPC" (vendor secHw): two banks of {status, set unsecure, set secure} */
#define SECHW_LS_SET_UNSECURE	0x04
#define SECHW_MS_SET_UNSECURE	0x10
/* chip control, key scan, touch screen, UART0, UART1, watchdog, SPUM, DDR2 */
#define SECHW_LS_VENDOR_MASK	0x00ff
#define SECHW_LS_ESW		0x0200
/* SPU, PKA, RNG, RTC, OTP, boot, MPU, TZ control, interrupt controller */
#define SECHW_MS_VENDOR_MASK	0x01ff

/* PL192-style interrupt controllers */
#define VIC_INT_ENABLE_CLEAR	0x14

/* DesignWare AHB DMA controller: what switching it off takes */
#define DW_MASK_TFR		0x310	/* five mask registers, 8 bytes apart */
#define DW_CLEAR_TFR		0x338	/* five clear registers, likewise */
#define DW_DMA_CFG		0x398
#define DW_CH_EN		0x3a0
#define   DW_ALL_CH_CLR			0xff00	/* write enables, bits clear */

/* DSP "VINTC" block (drivers/remoteproc/bcmring_vpm.c) */
#define VINTC_HALT		0x1c	/* 1 = core halted */

#define BCMRING_NR_VICS		3

static void __iomem *bcmring_chipc;
static void __iomem *bcmring_aram;
static void __iomem *bcmring_vics[BCMRING_NR_VICS];

static void __iomem *bcmring_iomap(const char *compatible)
{
	struct device_node *np;
	void __iomem *base;

	np = of_find_compatible_node(NULL, NULL, compatible);
	if (!np) {
		pr_err("bcmring: no %s node\n", compatible);
		return NULL;
	}
	base = of_iomap(np, 0);
	if (!base) {
		struct resource res;
		int ret = of_address_to_resource(np, 0, &res);

		pr_err("bcmring: cannot map %pOF (resource %d, %pR)\n", np,
		       ret, ret ? NULL : &res);
	}
	of_node_put(np);
	return base;
}

/*
 * "bcmring.coldsim" on the command line: for testing a kernel that is
 * started from a running system (RAM boot) against something closer to the
 * state a boot loader hands over.  It pulses the soft resets and switches
 * off the bus gates and clocks of the peripherals that have drivers here
 * and that a boot loader has no use for, so that a driver relying on state
 * left by another kernel shows itself.  The core blocks (timers, interrupt
 * controllers, GPIO, UART A, NAND, memory) are left alone: pulsing every
 * reset the vendor asserts before a chip reset hangs the kernel at once.
 */
static bool bcmring_coldsim __initdata;

static bool bcmring_coldsim_panic __initdata;

static int __init bcmring_coldsim_setup(char *arg)
{
	bcmring_coldsim = true;
	/* "bcmring.coldsim=panic": also panic while drivers probe. */
	bcmring_coldsim_panic = arg && !strcmp(arg, "panic");
	return 0;
}
early_param("bcmring.coldsim", bcmring_coldsim_setup);

static int __init bcmring_coldsim_panic_init(void)
{
	if (bcmring_coldsim_panic)
		panic("bcmring: coldsim: test panic during driver initialization");
	return 0;
}
device_initcall(bcmring_coldsim_panic_init);

static void __init bcmring_simulate_cold_state(void)
{
	/* SPI host, LCD, RNG, LED matrix, keypad */
	const u32 rst1 = BIT(12) | BIT(22) | BIT(24) | BIT(26) | BIT(27);
	/* APM (bits 32 and 45), Ethernet switch clock and core (37, 38) */
	const u32 rst2 = BIT(0) | BIT(13) | BIT(5) | BIT(6);
	/* DMA0, DMA1, switch, LCD, keypad, LED matrix, SPI host */
	const u32 gates = BIT(5) | BIT(6) | BIT(8) | BIT(14) | BIT(15) |
			  BIT(16) | BIT(22);
	/* switch, switch 125 MHz, SPI, APM100, LCD, APM200 */
	static const u16 clk_off[] __initconst = {
		0x0c, 0x14, 0x24, 0x44, 0x54, 0x58,
	};
	unsigned int i;
	u32 val;

	pr_warn("bcmring: coldsim: pulsing peripheral resets, gating peripheral clocks\n");
	for (i = 0; i < ARRAY_SIZE(clk_off); i++) {
		val = readl(bcmring_chipc + clk_off[i]);
		writel(val & ~BIT(16), bcmring_chipc + clk_off[i]);
	}
	val = readl(bcmring_chipc + CHIPC_SOFT_RESET1);
	writel(val | rst1, bcmring_chipc + CHIPC_SOFT_RESET1);
	readl(bcmring_chipc + CHIPC_SOFT_RESET1);
	writel(val & ~rst1, bcmring_chipc + CHIPC_SOFT_RESET1);
	val = readl(bcmring_chipc + CHIPC_SOFT_RESET2);
	writel(val | rst2, bcmring_chipc + CHIPC_SOFT_RESET2);
	readl(bcmring_chipc + CHIPC_SOFT_RESET2);
	writel(val & ~rst2, bcmring_chipc + CHIPC_SOFT_RESET2);

	writel(readl(bcmring_chipc + CHIPC_BUSINTF_CLK) & ~gates,
	       bcmring_chipc + CHIPC_BUSINTF_CLK);
}

static void __init bcmring_set_unsecure(void)
{
	void __iomem *tzpc;
	u32 val;

	bcmring_chipc = bcmring_iomap("brcm,bcmring-chipc");
	tzpc = bcmring_iomap("brcm,bcmring-tzpc");
	if (!bcmring_chipc || !tzpc) {
		pr_err("bcmring: cannot map chipc/tzpc; interrupts of secure peripherals will not arrive\n");
		return;
	}

	if (bcmring_coldsim)
		bcmring_simulate_cold_state();

	/* Not under the clock driver's lock: nothing else runs yet. */
	/*
	 * VRAM: the DSP shared RAM also holds the ramoops console, which is
	 * probed before the clock driver.  The gate is on after boot1 (its
	 * first stack is there); this makes that independent of boot2.
	 */
	writel(readl(bcmring_chipc + CHIPC_BUSINTF_CLK) | CHIPC_BUSINTF_SPU |
	       CHIPC_BUSINTF_VRAM, bcmring_chipc + CHIPC_BUSINTF_CLK);

	/*
	 * Pulse the LCD controller's soft reset.  A chip reset leaves it
	 * reset already; a kernel started from a running system (kexec-style
	 * RAM boot) can inherit a controller stopped in mid-frame that never
	 * fetches again (current-address register frozen, no next-base
	 * interrupt), which the PL111 driver does not recover from.  Seen on
	 * a D60; clearing it by this reset was tested there.
	 */
	val = readl(bcmring_chipc + CHIPC_SOFT_RESET1);
	writel(val | CHIPC_SOFT_RESET_LCD, bcmring_chipc + CHIPC_SOFT_RESET1);
	readl(bcmring_chipc + CHIPC_SOFT_RESET1);
	writel(val & ~CHIPC_SOFT_RESET_LCD, bcmring_chipc + CHIPC_SOFT_RESET1);

	writel(SECHW_LS_VENDOR_MASK | SECHW_LS_ESW,
	       tzpc + SECHW_LS_SET_UNSECURE);
	writel(SECHW_MS_VENDOR_MASK, tzpc + SECHW_MS_SET_UNSECURE);

	iounmap(tzpc);
}

static void __init bcmring_init_irq(void)
{
	/* Must precede interrupt controller setup; needs ioremap(). */
	bcmring_set_unsecure();
	irqchip_init();
}

/*
 * Runs from ARAM.  r0 = virtual base of chip control, r1 = value for
 * SoftReset1.  Position independent.  The NOP runs reproduce the vendor
 * routine's settling delays.
 */
asm(
"	.pushsection .text, \"ax\"\n"
"	.arm\n"
"	.align	3\n"
"	.type	bcmring_reset_stub, %function\n"
"bcmring_reset_stub:\n"
"	.rept	4\n"
"	nop\n"
"	.endr\n"
"	ldr	r3, [r0, #0x08]\n"		/* ARM clock */
"	orr	r3, r3, #0x20000\n"		/* bypass: run from the crystal */
"	str	r3, [r0, #0x08]\n"
"	.rept	20\n"
"	nop\n"
"	.endr\n"
"	str	r1, [r0, #0x80]\n"		/* SoftReset1: chip reset */
"1:	b	1b\n"
"	.size	bcmring_reset_stub, . - bcmring_reset_stub\n"
"bcmring_reset_stub_end:\n"
"	.popsection\n"
);

extern void bcmring_reset_stub(void __iomem *chipc, u32 reset);
extern const char bcmring_reset_stub_end[];

static int bcmring_restart(struct notifier_block *nb, unsigned long mode,
			   void *cmd)
{
	void (*stub)(void __iomem *chipc, u32 reset);
	size_t size;
	unsigned int i;
	u32 val;

	if (!bcmring_chipc || !bcmring_aram)
		return NOTIFY_DONE;

	local_irq_disable();
	local_fiq_disable();

	/* Mask every source on all three interrupt controllers. */
	for (i = 0; i < ARRAY_SIZE(bcmring_vics); i++) {
		if (bcmring_vics[i])
			writel(~0U, bcmring_vics[i] + VIC_INT_ENABLE_CLEAR);
	}

	/* Put the UART and SPI clocks on the crystal, as the boot ROM expects. */
	val = readl(bcmring_chipc + CHIPC_UART_CLK);
	writel(val | CHIPC_CLK_BYPASS, bcmring_chipc + CHIPC_UART_CLK);
	val = readl(bcmring_chipc + CHIPC_SPI_CLK);
	writel(val | CHIPC_CLK_BYPASS, bcmring_chipc + CHIPC_SPI_CLK);

	size = bcmring_reset_stub_end - (const char *)bcmring_reset_stub;
	stub = fncpy(bcmring_aram, &bcmring_reset_stub, size);
	flush_icache_range((unsigned long)bcmring_aram,
			   (unsigned long)bcmring_aram + size);

	/*
	 * Hold every block in reset except chip soft/warm, ARM, ARM hold, and
	 * the watchdog (vendor "force reset" path: SoftReset1 |= ~0x4f,
	 * SoftReset2 |= 0x3fe7).
	 */
	val = readl(bcmring_chipc + CHIPC_SOFT_RESET1);
	writel(val | ~0x4fU, bcmring_chipc + CHIPC_SOFT_RESET1);
	val = readl(bcmring_chipc + CHIPC_SOFT_RESET2);
	writel(val | 0x3fe7, bcmring_chipc + CHIPC_SOFT_RESET2);

	/*
	 * TODO: the vendor code uses the warm reset (bit 1, with SoftReset2
	 * |= 0x3f87, leaving the Ethernet switch alone) only for a requested
	 * 'h' reboot while a vendor-specific flag is set; everything else is
	 * a chip soft reset.
	 */
	stub(bcmring_chipc, CHIPC_SOFT_RESET_CHIP_SOFT);

	return NOTIFY_DONE;
}

static struct notifier_block bcmring_restart_nb = {
	.notifier_call = bcmring_restart,
	.priority = 192,
};

#ifdef CONFIG_KEXEC_CORE
/*
 * kexec.  A boot loader hands over with the DSP halted, no DMA channel
 * running, and no interrupt source enabled; a running system does not, and
 * a DMA channel left running (the Ethernet receiver in particular) would
 * write into the memory of the next kernel while it starts.  The driver
 * shutdown callbacks do not cover this: the Ethernet driver and the DSP
 * driver have none, and the DSP firmware programs channel 0 of the first
 * controller by itself.
 *
 * This runs as the last step before the jump (a syscore shutdown callback,
 * after every driver's) and only for kexec: a restart goes through a chip
 * reset, which leaves all of this in its reset state anyway.  The steps
 * and their order are those of the loader that started this kernel out of
 * the stock firmware in every RAM boot of the bring-up (halt the DSP,
 * switch both DMA controllers off, mask every interrupt source); a
 * controller whose bus gate is off is not touched, since its registers do
 * not answer then.  The watchdog is deliberately left running: it is what
 * ends a next kernel that hangs.
 */
static void __iomem *bcmring_kexec_dma[2];
static void __iomem *bcmring_kexec_vintc;

static void bcmring_kexec_dma_off(void __iomem *base)
{
	unsigned int i;

	writel(DW_ALL_CH_CLR, base + DW_CH_EN);
	writel(0, base + DW_DMA_CFG);
	for (i = 0; i < 5; i++)
		writel(DW_ALL_CH_CLR, base + DW_MASK_TFR + 8 * i);
	for (i = 0; i < 1000000 && (readl(base + DW_DMA_CFG) & 1); i++)
		cpu_relax();
	for (i = 0; i < 5; i++)
		writel(0xff, base + DW_CLEAR_TFR + 8 * i);
}

static void bcmring_kexec_quiesce(void *data)
{
	static const u32 gate[] = {
		CHIPC_BUSINTF_DMAC0, CHIPC_BUSINTF_DMAC1,
	};
	unsigned long flags;
	unsigned int i;
	u32 gates;

	if (!kexec_in_progress || !bcmring_chipc)
		return;

	pr_info("bcmring: kexec: halting the DSP, DMA off, interrupts masked\n");
	/* No handler may run between the steps; none can after the last. */
	local_irq_save(flags);
	if (bcmring_kexec_vintc)
		writel(1, bcmring_kexec_vintc + VINTC_HALT);
	gates = readl(bcmring_chipc + CHIPC_BUSINTF_CLK);
	for (i = 0; i < ARRAY_SIZE(bcmring_kexec_dma); i++) {
		if (bcmring_kexec_dma[i] && (gates & gate[i]))
			bcmring_kexec_dma_off(bcmring_kexec_dma[i]);
	}
	for (i = 0; i < ARRAY_SIZE(bcmring_vics); i++) {
		if (bcmring_vics[i])
			writel(~0U, bcmring_vics[i] + VIC_INT_ENABLE_CLEAR);
	}
	local_irq_restore(flags);
}

static const struct syscore_ops bcmring_kexec_syscore_ops = {
	.shutdown = bcmring_kexec_quiesce,
};

static struct syscore bcmring_kexec_syscore = {
	.ops = &bcmring_kexec_syscore_ops,
};

/* Mapped at machine init: the callback must not sleep. */
static void __init bcmring_kexec_init(void)
{
	struct device_node *np;
	unsigned int n = 0;

	/* Both controllers, the one the Ethernet driver owns included. */
	for_each_compatible_node(np, NULL, "snps,dma-spear1340") {
		if (n < ARRAY_SIZE(bcmring_kexec_dma))
			bcmring_kexec_dma[n++] = of_iomap(np, 0);
	}
	/* The DSP node's compatible is per SoC; its name is not. */
	np = of_find_node_by_name(NULL, "dsp");
	if (np) {
		int i = of_property_match_string(np, "reg-names", "vintc");

		if (i >= 0)
			bcmring_kexec_vintc = of_iomap(np, i);
		of_node_put(np);
	}
	register_syscore(&bcmring_kexec_syscore);
}
#else
static inline void bcmring_kexec_init(void) { }
#endif

/*
 * At machine init (an arch initcall), not later: a panic while drivers
 * probe must find the restart handler in place, or "panic=" ends in
 * "Reboot failed -- System halted" with the watchdog off (boot2 hands over
 * with it disabled), and the phone stays dead until power-cycled.  Seen on
 * a D60.
 */
static void __init bcmring_init_machine(void)
{
	static const char * const vics[] __initconst = {
		"brcm,bcmring-intc", "brcm,bcmring-sintc",
	};
	struct device_node *np;
	struct resource res;
	unsigned int i, n = 0;

	/* Mapped here because the restart handler must not sleep. */
	for (i = 0; i < ARRAY_SIZE(vics); i++) {
		for_each_compatible_node(np, NULL, vics[i]) {
			if (n < ARRAY_SIZE(bcmring_vics))
				bcmring_vics[n++] = of_iomap(np, 0);
		}
	}

	bcmring_kexec_init();

	np = of_find_compatible_node(NULL, NULL, "brcm,bcmring-aram");
	if (!np || of_address_to_resource(np, 0, &res)) {
		of_node_put(np);
		return;
	}
	of_node_put(np);

	/* One page is plenty; the stub is under 128 bytes. */
	bcmring_aram = __arm_ioremap_exec(res.start, SZ_4K, false);
	if (!bcmring_aram)
		return;

	register_restart_handler(&bcmring_restart_nb);
}

static const char * const bcmring_dt_compat[] __initconst = {
	"brcm,bcm11107",
	"brcm,bcm11109",
	"brcm,bcm11170",
	"brcm,bcm11188",
	"brcm,bcm11189",
	NULL,
};

DT_MACHINE_START(BCMRING_DT, "Broadcom BCMRING (BCM11107 family)")
	.init_irq	= bcmring_init_irq,
	.init_machine	= bcmring_init_machine,
	.dt_compat	= bcmring_dt_compat,
MACHINE_END
