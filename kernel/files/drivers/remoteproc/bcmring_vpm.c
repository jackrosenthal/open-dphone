// SPDX-License-Identifier: GPL-2.0-only
/*
 * Broadcom BCMRING (BCM11107 family: BCM11109/11170/11188/11189) voice DSP
 * ("VPM", a CEVA TeakLite-III core): loader, start/stop, and job transport.
 *
 * DRAFT.  Written from an interface description recovered by static analysis
 * of vendor binaries and by read-only observation of phones running the
 * stock firmware.  Compile tested only; it has never been run on hardware.
 * Comments say "verified" for what was read out of a vendor binary or
 * measured on a phone, "inferred" for conclusions drawn from that, and
 * "GUESS" for choices this driver had to make where nothing is known.
 *
 * What the DSP is: a compute coprocessor.  The host sends it synchronous
 * "jobs" (codec frames, echo-canceller frames, band splitting, and their
 * configuration) and gets each result back before sending the next.  It owns
 * no audio hardware and no network path.
 *
 * Pieces:
 *
 *  - remoteproc: firmware is the vendor's flat record file (address, size,
 *    payload; no header, no checksum), loaded into the DSP's program RAM, its
 *    data RAM, and the low DDR that the devicetree keeps away from Linux.
 *    The firmware cannot be redistributed; it comes from the phone's own
 *    stock root file system through request_firmware().  Nothing is started
 *    automatically.
 *
 *  - transport: a mailbox (four 16-bit words) and four rings in the 64 KiB
 *    shared RAM, one interrupt in each direction.  One job is in flight at
 *    any time, system-wide (verified in the vendor code, which serializes on
 *    one semaphore).
 *
 *  - interface: bcmring_vpm_job() for kernel users and the misc device
 *    "bcmring-vpm" for userspace (include/uapi/linux/bcmring_vpm.h).
 *
 * Not implemented: the asynchronous event path (mailbox word 0xab03, rings 3
 * and 4), which only the sub-band canceller can use and which was never
 * seen in use; events are logged, freed, and counted.  Suspend/resume.
 *
 * DMA: the firmware programs channel 0 of DMA controller 0 by itself for
 * block copies (inferred from live channel dumps).  Mainline dw_dmac
 * switches the whole controller off at probe and whenever its last channel
 * is released, so this driver holds that channel through the dmaengine for
 * as long as it is bound ("dmas").  That keeps the controller enabled and
 * keeps the channel away from other clients.  It never submits a transfer.
 * Not solved: while another dw_dmac channel is active, dw_dmac's interrupt
 * path unmasks and clears the status bits of all channels, this one
 * included; whether the firmware minds is unknown.
 */

#include <linux/bitfield.h>
#include <linux/bits.h>
#include <linux/clk.h>
#include <linux/completion.h>
#include <linux/delay.h>
#include <linux/dmaengine.h>
#include <linux/firmware.h>
#include <linux/interrupt.h>
#include <linux/io.h>
#include <linux/iopoll.h>
#include <linux/mfd/syscon.h>
#include <linux/miscdevice.h>
#include <linux/mod_devicetable.h>
#include <linux/module.h>
#include <linux/mutex.h>
#include <linux/of.h>
#include <linux/of_platform.h>
#include <linux/of_reserved_mem.h>
#include <linux/platform_device.h>
#include <linux/regmap.h>
#include <linux/remoteproc.h>
#include <linux/remoteproc/bcmring_vpm.h>
#include <linux/slab.h>
#include <linux/uaccess.h>
#include <linux/unaligned.h>
#include <linux/workqueue.h>

#include <uapi/linux/bcmring_vpm.h>

#define vpm_rd16(p)			readw(p)
#define vpm_wr16(v, p)			writew(v, p)
#define vpm_copy_in(p, src, len)	memcpy_toio(p, src, len)
#define vpm_copy_out(dst, p, len)	memcpy_fromio(dst, p, len)
#include "bcmring_vpm_proto.h"

/*
 * Chip control (syscon).  All verified in the vendor kernel (vpmHw_* and
 * chipcHw_vpmPhaseAlign).  The clock/reset driver exposes the VPM clock as
 * read-only and has no gate for it, so the words are driven from here in
 * the vendor's exact order instead of through the clock and reset
 * frameworks.
 */
#define CHIPC_VPM_CLK		0x10
#define  VPM_CLK_PHASE		GENMASK(13, 8)	/* phase control value */
#define  VPM_CLK_PHASE_UPDATE	BIT(15)		/* toggle to apply the value */
#define  VPM_CLK_ENABLE		BIT(16)
#define  VPM_CLK_PHASE_STATUS	BIT(23)		/* phase comparator, read only */
#define CHIPC_SOFT_RESET1	0x80
#define  VPM_RESET_HOLDS	(BIT(31) | BIT(30))	/* global hold, hold */
#define CHIPC_SOFT_RESET2	0x84
/*
 * The vendor sets bit 1 here after a successful phase alignment.  Register
 * and bit names ("spare 1", "VPM bus access enable") are recalled from the
 * old public mach-bcmring headers and were not re-checked against them.
 */
#define CHIPC_SPARE1		0x88
#define  SPARE1_VPM_BUS_ACCESS	BIT(1)

#define VPM_PHASE_MAX_STEPS	0xfffe
#define VPM_PHASE_RETRIES	10

/*
 * "VINTC" block.  Only these two words are known (verified by use); the
 * rest of the block is never touched.
 */
#define VINTC_HOST_IRQ		0x0c	/* write 1: interrupt the DSP */
#define VINTC_HALT		0x1c	/* 1 = core halted, 0 = running */

/* shared RAM */
#define VRAM_CMD		0x00	/* host to DSP command */
#define VRAM_CMD_ARG		0x02	/* channel */
#define VRAM_STATUS		0x04	/* DSP to host status */
#define VRAM_STATUS_ARG		0x06	/* channel, or assertion dump address */
#define VRAM_MAGIC		0x08	/* 0x1234, written once by the host */
#define VRAM_RINGS		0x10

#define VPM_MAGIC		0x1234
#define VPM_MSG_JOB		0xab00	/* job submitted / result ready */
#define VPM_MSG_IDLE		0xab01	/* acknowledged */
#define VPM_MSG_ASSERT		0xab02	/* DSP assertion (DSP to host only) */
#define VPM_MSG_ASYNC		0xab03	/* asynchronous event / response */

enum {
	VPM_RING_RESULT,	/* DSP to host: job results */
	VPM_RING_JOB,		/* host to DSP: job submissions */
	VPM_RING_EVENT,		/* DSP to host: asynchronous events */
	VPM_RING_RESPONSE,	/* host to DSP: asynchronous responses */
	VPM_NUM_RINGS
};

#define VPM_JOB_TIMEOUT_MS	1000	/* the vendor's wait for a result */
#define VPM_JOB_TIMEOUT_MAX_MS	10000
#define VPM_IDLE_TIMEOUT_US	100000
#define VPM_BOOT_SETTLE_MS	50	/* the vendor's sleep after "run" */
/* GUESS: the vendor formats a register dump whose layout is not decoded. */
#define VPM_ASSERT_DUMP_WORDS	64

#define VPM_FW_NAME		"dspapp1107_lcc.bin"

enum vpm_event {
	VPM_EV_NONE,
	VPM_EV_RESULT,
	VPM_EV_ASYNC,
	VPM_EV_FAILED,
};

/**
 * struct bcmring_vpm_family - what differs between the two firmware families
 * @id: BCMRING_VPM_FAMILY_*
 * @ring_words: size of each of the four rings in 16-bit words
 * @channels: size of the vendor's channel table; channel numbers stay below
 * @ddr_limit: the vendor loader refuses DDR records that end above this
 * @has_band: the band split/combine client exists
 */
struct bcmring_vpm_family {
	u32 id;
	u16 ring_words;
	u16 channels;
	u32 ddr_limit;
	bool has_band;
};

struct bcmring_vpm {
	struct device *dev;
	struct rproc *rproc;
	const struct bcmring_vpm_family *family;
	struct regmap *chipc;
	void __iomem *vram;
	void __iomem *vintc;
	void __iomem *pram;
	void __iomem *dram;
	struct resource pram_res;
	struct resource dram_res;
	struct resource ddr_res;
	struct dma_chan *dma;
	int irq;

	/* core power: clock, phase alignment, halt word */
	struct mutex power_lock;
	bool running;
	struct delayed_work halt_work;

	/* one job at a time; also guards online/failed and the rings */
	struct mutex job_lock;
	bool online;
	bool failed;
	/* set by the interrupt handler: the DSP reported a failure */
	bool dead;
	bool overdue;		/* the last job timed out; its result may come */
	struct vpm_ring rings[VPM_NUM_RINGS];
	struct completion event;
	/* written by the interrupt handler, read by the waiter */
	enum vpm_event ev;
	u16 ev_arg;

	struct work_struct assert_work;
	s16 assert_arg;

	u32 jobs;
	u32 timeouts;
	u32 asserts;
	u32 async_events;
	u32 stale_results;

	struct miscdevice misc;
};

/*
 * Ring size and channel-table size are compiled into each family's firmware
 * and host modules (verified in the host modules; confirmed for the ring
 * size by where the rings sit in the shared RAM of running phones).
 */
static const struct bcmring_vpm_family bcmring_vpm_old_family = {
	.id = BCMRING_VPM_FAMILY_OLD,
	.ring_words = 0x1000,
	.channels = 16,
	.ddr_limit = 0x200000,
};

static const struct bcmring_vpm_family bcmring_vpm_new_family = {
	.id = BCMRING_VPM_FAMILY_NEW,
	.ring_words = 0x1500,
	.channels = 32,
	.ddr_limit = 0x400000,
	.has_band = true,
};

static const struct of_device_id bcmring_vpm_of_match[] = {
	{ .compatible = "brcm,bcm11109-vpm", .data = &bcmring_vpm_old_family },
	{ .compatible = "brcm,bcm11170-vpm", .data = &bcmring_vpm_old_family },
	{ .compatible = "brcm,bcm11188-vpm", .data = &bcmring_vpm_new_family },
	{ .compatible = "brcm,bcm11189-vpm", .data = &bcmring_vpm_new_family },
	{ }
};
MODULE_DEVICE_TABLE(of, bcmring_vpm_of_match);

static unsigned int halt_delay_ms = 3000;
module_param(halt_delay_ms, uint, 0644);
MODULE_PARM_DESC(halt_delay_ms,
		 "Halt the DSP and gate its clock this long after the last job (default 3000, as the vendor)");

static bool no_halt;
module_param(no_halt, bool, 0644);
MODULE_PARM_DESC(no_halt, "Keep the DSP running between jobs");

/* ---- core power ---------------------------------------------------- */

/* Apply one phase value and return the comparator output that follows. */
static bool bcmring_vpm_phase_step(struct bcmring_vpm *vpm, unsigned int phase)
{
	unsigned int val;

	regmap_update_bits(vpm->chipc, CHIPC_VPM_CLK, VPM_CLK_PHASE,
			   FIELD_PREP(VPM_CLK_PHASE, phase));
	udelay(1);
	regmap_read(vpm->chipc, CHIPC_VPM_CLK, &val);
	regmap_update_bits(vpm->chipc, CHIPC_VPM_CLK, VPM_CLK_PHASE_UPDATE,
			   ~val);
	regmap_read(vpm->chipc, CHIPC_VPM_CLK, &val);

	return val & VPM_CLK_PHASE_STATUS;
}

/*
 * Align the DSP clock to the bus clock.  This is a transcription of the
 * search the vendor kernel runs each time it enables the clock (verified:
 * vpmSwPhaseAlign and chipcHw_vpmPhaseAlign in the D60 kernel); what the
 * steps achieve electrically is not documented.  In short: move the phase
 * until the comparator output falls from 1 to 0, check that the edge is
 * clean (the output stays 0 for five steps up and three steps back, and is
 * 1 again five steps further back), then return to the edge and stop one
 * step below it.  Up to ten attempts; like the vendor, an exhausted retry
 * count still finishes at the last position.  Only running out of steps is
 * an error.
 *
 * Returns the number of steps taken or a negative error.
 */
static int bcmring_vpm_phase_align(struct bcmring_vpm *vpm)
{
	unsigned int steps = 0, phase, val, retry, i;
	bool prev, cur;

	for (retry = 0; retry < VPM_PHASE_RETRIES; retry++) {
		regmap_read(vpm->chipc, CHIPC_VPM_CLK, &val);
		phase = FIELD_GET(VPM_CLK_PHASE, val);
		prev = val & VPM_CLK_PHASE_STATUS;

		for (;;) {
			cur = bcmring_vpm_phase_step(vpm, phase);
			steps++;
			phase = (cur ? phase + 1 : phase - 1) & 0x3f;
			if (prev && !cur)
				break;
			if (steps > VPM_PHASE_MAX_STEPS)
				return -EIO;
			prev = cur;
		}
		if (steps > VPM_PHASE_MAX_STEPS)
			return -EIO;

		for (i = 0; i < 5 && !cur; i++) {
			phase = (phase + 1) & 0x3f;
			cur = bcmring_vpm_phase_step(vpm, phase);
			steps++;
		}
		if (steps > VPM_PHASE_MAX_STEPS)
			return -EIO;
		if (i != 5)
			continue;

		for (i = 0; i < 3 && !cur; i++) {
			phase = (phase - 1) & 0x3f;
			cur = bcmring_vpm_phase_step(vpm, phase);
			steps++;
		}
		if (steps > VPM_PHASE_MAX_STEPS)
			return -EIO;
		if (i != 3)
			continue;

		for (i = 0; i < 5; i++) {
			phase = (phase - 1) & 0x3f;
			cur = bcmring_vpm_phase_step(vpm, phase);
		}
		steps += 5;
		if (steps > VPM_PHASE_MAX_STEPS)
			return -EIO;
		if (!cur)
			continue;

		/*
		 * The vendor compares the whole previous register value with
		 * the masked status bit here, which can never match, so the
		 * loop ends on the first step that reads 0.
		 */
		do {
			cur = bcmring_vpm_phase_step(vpm, phase);
			steps++;
			phase = (cur ? phase + 1 : phase - 1) & 0x3f;
			if (steps > VPM_PHASE_MAX_STEPS)
				return -EIO;
		} while (cur);
		break;
	}

	regmap_read(vpm->chipc, CHIPC_VPM_CLK, &val);
	phase = (FIELD_GET(VPM_CLK_PHASE, val) - 1) & 0x3f;
	regmap_update_bits(vpm->chipc, CHIPC_VPM_CLK, VPM_CLK_PHASE,
			   FIELD_PREP(VPM_CLK_PHASE, phase));
	regmap_read(vpm->chipc, CHIPC_VPM_CLK, &val);
	regmap_update_bits(vpm->chipc, CHIPC_VPM_CLK, VPM_CLK_PHASE_UPDATE,
			   ~val);

	regmap_set_bits(vpm->chipc, CHIPC_SPARE1, SPARE1_VPM_BUS_ACCESS);

	return steps;
}

static int bcmring_vpm_clock_on(struct bcmring_vpm *vpm)
{
	int ret;

	regmap_set_bits(vpm->chipc, CHIPC_VPM_CLK, VPM_CLK_ENABLE);
	ret = bcmring_vpm_phase_align(vpm);
	if (ret < 0) {
		/*
		 * The vendor only prints a message and runs the core anyway.
		 * GUESS: refusing is the safer course for a first bring-up.
		 */
		dev_err(vpm->dev, "clock phase alignment failed\n");
		return ret;
	}
	dev_dbg(vpm->dev, "clock phase aligned in %d steps\n", ret);

	return 0;
}

/* Halt the core, then gate its clock (verified order, vpmHw_Halt). */
static void bcmring_vpm_halt(struct bcmring_vpm *vpm)
{
	mutex_lock(&vpm->power_lock);
	writel(1, vpm->vintc + VINTC_HALT);
	regmap_clear_bits(vpm->chipc, CHIPC_VPM_CLK, VPM_CLK_ENABLE);
	vpm->running = false;
	mutex_unlock(&vpm->power_lock);
}

/* Clock on, align, release the halt (verified order, vpmHw_Run). */
static int bcmring_vpm_run(struct bcmring_vpm *vpm)
{
	int ret = 0;

	cancel_delayed_work_sync(&vpm->halt_work);

	mutex_lock(&vpm->power_lock);
	if (!vpm->running) {
		ret = bcmring_vpm_clock_on(vpm);
		if (!ret) {
			writel(0, vpm->vintc + VINTC_HALT);
			vpm->running = true;
		} else {
			regmap_clear_bits(vpm->chipc, CHIPC_VPM_CLK,
					  VPM_CLK_ENABLE);
		}
	}
	mutex_unlock(&vpm->power_lock);

	return ret;
}

static void bcmring_vpm_halt_work(struct work_struct *work)
{
	struct bcmring_vpm *vpm = container_of(to_delayed_work(work),
					       struct bcmring_vpm, halt_work);

	bcmring_vpm_halt(vpm);
}

/* The vendor halts the core three seconds after the last result. */
static void bcmring_vpm_idle(struct bcmring_vpm *vpm)
{
	if (!no_halt)
		mod_delayed_work(system_wq, &vpm->halt_work,
				 msecs_to_jiffies(halt_delay_ms));
}

/* ---- transport ----------------------------------------------------- */

static irqreturn_t bcmring_vpm_irq(int irq, void *data)
{
	struct bcmring_vpm *vpm = data;
	u16 status = readw(vpm->vram + VRAM_STATUS);
	u16 arg = readw(vpm->vram + VRAM_STATUS_ARG);

	/*
	 * The DSP raises this line through the interrupt controller's
	 * soft-interrupt register (inferred from the vendor acknowledging
	 * with a write to the soft-interrupt-clear register).  The VIC
	 * irqchip's acknowledge has already done that same write.
	 */
	switch (status) {
	case VPM_MSG_JOB:
		vpm->ev = VPM_EV_RESULT;
		break;
	case VPM_MSG_ASYNC:
		vpm->ev = VPM_EV_ASYNC;
		break;
	case VPM_MSG_ASSERT:
		/* the status word is left in place, as the vendor does */
		vpm->assert_arg = arg;
		WRITE_ONCE(vpm->dead, true);
		vpm->ev = VPM_EV_FAILED;
		complete(&vpm->event);
		schedule_work(&vpm->assert_work);
		return IRQ_HANDLED;
	case VPM_MSG_IDLE:
		/* nothing posted */
		return IRQ_HANDLED;
	default:
		/* the vendor asserts on this */
		dev_err_ratelimited(vpm->dev,
				    "bad mailbox status 0x%04x (argument 0x%04x)\n",
				    status, arg);
		WRITE_ONCE(vpm->dead, true);
		vpm->ev = VPM_EV_FAILED;
		complete(&vpm->event);
		rproc_report_crash(vpm->rproc, RPROC_FATAL_ERROR);
		return IRQ_HANDLED;
	}

	vpm->ev_arg = arg;
	complete(&vpm->event);
	writew(VPM_MSG_IDLE, vpm->vram + VRAM_STATUS);

	return IRQ_HANDLED;
}

static void bcmring_vpm_assert_work(struct work_struct *work)
{
	struct bcmring_vpm *vpm = container_of(work, struct bcmring_vpm,
					       assert_work);
	long off = 2 * (long)vpm->assert_arg;
	u16 dump[VPM_ASSERT_DUMP_WORDS];
	unsigned int i;

	vpm->asserts++;
	dev_err(vpm->dev,
		"DSP assertion; register dump at data RAM word address %d\n",
		vpm->assert_arg);

	/*
	 * Verified: the argument is a signed word address into the data RAM.
	 * The dump's layout is not decoded; the words are printed raw.
	 */
	if (off >= 0 &&
	    off + sizeof(dump) <= resource_size(&vpm->dram_res)) {
		for (i = 0; i < ARRAY_SIZE(dump); i++)
			dump[i] = readw(vpm->dram + off + 2 * i);
		print_hex_dump(KERN_ERR, "vpm dump: ", DUMP_PREFIX_OFFSET, 16,
			       2, dump, sizeof(dump), false);
	}

	rproc_report_crash(vpm->rproc, RPROC_FATAL_ERROR);
}

/*
 * An asynchronous event: the sub-band canceller asking the host to read or
 * write a block of host-side state in the middle of a job (inferred).  The
 * event layout and the expected response are not decoded, and no event was
 * seen in any trace of a stock phone.  Log it and free it; the job that
 * caused it will most likely time out.
 */
static void bcmring_vpm_async_event(struct bcmring_vpm *vpm)
{
	struct vpm_ring *ring = &vpm->rings[VPM_RING_EVENT];
	void __iomem *payload;
	u16 words[16];
	int len, i;

	vpm->async_events++;
	len = vpm_ring_peek(ring, &payload);
	if (len <= 0) {
		dev_warn(vpm->dev, "asynchronous event without a frame (%d)\n",
			 len);
		return;
	}

	for (i = 0; i < len && i < ARRAY_SIZE(words); i++)
		words[i] = readw(payload + 2 * i);
	dev_warn(vpm->dev,
		 "asynchronous event of %d words on channel %u is not served: %*ph\n",
		 len, vpm->ev_arg, (int)(i * sizeof(words[0])), words);
	vpm_ring_free(ring);
}

/*
 * Drop what nobody waits for: results of jobs that timed out, and events
 * that arrived together with a result.  Returns the number of results
 * dropped or a negative error.
 */
static int bcmring_vpm_drain(struct bcmring_vpm *vpm)
{
	struct vpm_ring *ring = &vpm->rings[VPM_RING_RESULT];
	void __iomem *payload;
	int len, dropped = 0;

	while ((len = vpm_ring_peek(ring, &payload)) > 0) {
		vpm_ring_free(ring);
		vpm->stale_results++;
		dropped++;
		dev_warn_ratelimited(vpm->dev,
				     "dropped a stale result of %d words\n",
				     len);
	}
	if (len < 0)
		return len;

	ring = &vpm->rings[VPM_RING_EVENT];
	while ((len = vpm_ring_peek(ring, &payload)) > 0)
		bcmring_vpm_async_event(vpm);

	return len < 0 ? len : dropped;
}

static bool bcmring_vpm_client_ok(const struct bcmring_vpm *vpm, u16 client)
{
	if (client == VPM_CLIENT_BAND)
		return vpm->family->has_band;

	return vpm_hdr_words(client) != 0;
}

static unsigned int bcmring_vpm_max_body(const struct bcmring_vpm *vpm)
{
	return 2 * (vpm_ring_max_payload(&vpm->rings[VPM_RING_JOB]) -
		    VPM_HDR_MAX_WORDS);
}

/* Take the result frame apart.  Called with a frame of @len words. */
static int bcmring_vpm_take_result(struct bcmring_vpm *vpm,
				   struct bcmring_vpm_kjob *job,
				   void __iomem *payload, unsigned int len)
{
	u16 words[VPM_HDR_MAX_WORDS];
	struct vpm_job_hdr hdr;
	unsigned int i;
	size_t body;
	int n;

	for (i = 0; i < len && i < ARRAY_SIZE(words); i++)
		words[i] = readw(payload + 2 * i);

	/*
	 * A result repeats the channel, client, instance, codec, and opcode
	 * of its request (measured on frames left in the rings).
	 */
	n = vpm_hdr_decode(words, i, &hdr);
	if (n < 0 || hdr.channel != job->channel ||
	    hdr.client != job->client || hdr.instance != job->instance ||
	    hdr.opcode != job->opcode ||
	    (hdr.client == VPM_CLIENT_PVE && hdr.codec != job->codec)) {
		dev_err(vpm->dev,
			"result does not match the job (channel %u client %u instance %u opcode %u): %*ph\n",
			job->channel, job->client, job->instance, job->opcode,
			(int)(i * sizeof(words[0])), words);
		return -EPROTO;
	}

	/*
	 * The frame length, not the header's length word, says how much body
	 * there is (the DSP has been seen to leave that word 0).
	 */
	body = 2 * (len - n);
	job->arg1 = hdr.arg1;
	job->arg2 = hdr.arg2;
	job->reply_len_field = hdr.len;
	job->reply_len = body;
	if (job->reply && job->reply_max)
		memcpy_fromio(job->reply, payload + 2 * n,
			      min(body, job->reply_max));

	return 0;
}

/**
 * bcmring_vpm_job() - run one job on the DSP and wait for its result
 * @vpm: the DSP
 * @job: request and, on return, result
 *
 * Sleeps.  Jobs are serialized.  Return: 0 or a negative error (see the
 * UAPI header for their meaning).
 */
int bcmring_vpm_job(struct bcmring_vpm *vpm, struct bcmring_vpm_kjob *job)
{
	struct vpm_ring *jobs = &vpm->rings[VPM_RING_JOB];
	struct vpm_ring *results = &vpm->rings[VPM_RING_RESULT];
	unsigned long timeout, left;
	u16 words[VPM_HDR_MAX_WORDS];
	struct vpm_job_hdr hdr = {
		.channel = job->channel,
		.client = job->client,
		.instance = job->instance,
		.codec = job->codec,
		.opcode = job->opcode,
		.arg1 = job->arg1,
		.arg2 = job->arg2,
		.len = job->body_len,
	};
	void __iomem *payload;
	unsigned int n;
	bool drained;
	u16 val;
	int ret;

	if (!bcmring_vpm_client_ok(vpm, job->client) ||
	    job->channel >= vpm->family->channels ||
	    (job->body_len && !job->body))
		return -EINVAL;
	if (job->body_len > bcmring_vpm_max_body(vpm))
		return -EMSGSIZE;

	timeout = job->timeout_ms ?: VPM_JOB_TIMEOUT_MS;
	timeout = msecs_to_jiffies(min_t(unsigned long, timeout,
					 VPM_JOB_TIMEOUT_MAX_MS));

	ret = mutex_lock_killable(&vpm->job_lock);
	if (ret)
		return ret;

	if (!vpm->online) {
		ret = -ENODEV;
		goto unlock;
	}
	/* a failure may have been reported while no job was waiting */
	if (vpm->failed || READ_ONCE(vpm->dead)) {
		ret = -EIO;
		goto unlock;
	}

	/*
	 * The vendor writes the mailbox first and starts the clock second.
	 * Here the core is un-halted first, so that a command it has not
	 * acknowledged yet can still be acknowledged.
	 */
	ret = bcmring_vpm_run(vpm);
	if (ret)
		goto unlock;

	ret = bcmring_vpm_drain(vpm);
	if (ret < 0)
		goto failed;

	/*
	 * One job in flight, system-wide.  If the previous job timed out and
	 * its result has still not come, the DSP may be working on it: do
	 * not send another one on top.  Only a restart ends this state if
	 * the result never comes.
	 */
	if (vpm->overdue && !ret) {
		ret = -EBUSY;
		goto idle;
	}
	vpm->overdue = false;

	/*
	 * The DSP writes "idle" to the command word when it has taken the
	 * previous command (inferred; the host never writes that value).
	 * The vendor spins here for up to 65,535 reads and then asserts.
	 */
	ret = readw_poll_timeout(vpm->vram + VRAM_CMD, val,
				 val == VPM_MSG_IDLE, 20, VPM_IDLE_TIMEOUT_US);
	if (ret) {
		dev_err(vpm->dev, "DSP does not take commands (word 0x%04x)\n",
			val);
		goto idle;
	}
	/*
	 * The DSP works on the request in place and may publish that it has
	 * freed the frame a little after it posts the result (not known
	 * either way): give it the same grace as above.
	 */
	ret = read_poll_timeout(vpm_ring_drained, drained, drained, 20,
				VPM_IDLE_TIMEOUT_US, false, jobs);
	if (ret) {
		dev_err(vpm->dev, "DSP has not consumed the previous job\n");
		ret = -EBUSY;
		goto idle;
	}

	n = vpm_hdr_encode(&hdr, words);
	vpm->ev = VPM_EV_NONE;
	reinit_completion(&vpm->event);

	ret = vpm_ring_put(jobs, words, n, job->body, job->body_len);
	if (ret)
		goto idle;

	writew(job->channel, vpm->vram + VRAM_CMD_ARG);
	writew(VPM_MSG_JOB, vpm->vram + VRAM_CMD);
	writel(1, vpm->vintc + VINTC_HOST_IRQ);

	left = timeout;
	for (;;) {
		left = wait_for_completion_timeout(&vpm->event, left);
		if (!left || vpm->ev == VPM_EV_FAILED)
			break;
		if (vpm->ev == VPM_EV_ASYNC) {
			bcmring_vpm_async_event(vpm);
			continue;
		}
		/*
		 * The late interrupt of an earlier, timed-out job can wake
		 * us before our own result is there: keep waiting then.
		 */
		if (vpm_ring_peek(results, &payload))
			break;
	}

	if (vpm->ev == VPM_EV_FAILED) {
		ret = -EIO;
		goto failed;
	}

	ret = vpm_ring_peek(results, &payload);
	if (ret < 0)
		goto failed;
	if (!ret) {
		vpm->timeouts++;
		vpm->overdue = true;
		dev_warn_ratelimited(vpm->dev,
				     "no result for client %u opcode %u on channel %u\n",
				     job->client, job->opcode, job->channel);
		ret = -ETIMEDOUT;
		goto idle;
	}
	if (!left)
		dev_warn_ratelimited(vpm->dev,
				     "result arrived without an interrupt\n");

	ret = bcmring_vpm_take_result(vpm, job, payload, ret);
	vpm_ring_free(results);
	if (!ret)
		vpm->jobs++;

idle:
	bcmring_vpm_idle(vpm);
unlock:
	mutex_unlock(&vpm->job_lock);
	return ret;

failed:
	/*
	 * The rings cannot be trusted any more: stop until a restart, and tell
	 * the remoteproc core, so that its recovery can be used.
	 */
	vpm->failed = true;
	bcmring_vpm_idle(vpm);
	mutex_unlock(&vpm->job_lock);
	rproc_report_crash(vpm->rproc, RPROC_FATAL_ERROR);
	return ret;
}
EXPORT_SYMBOL_GPL(bcmring_vpm_job);

/**
 * bcmring_vpm_get() - look the DSP up by its devicetree node
 * @np: node of the DSP
 *
 * Return: the DSP with a reference held, -EPROBE_DEFER if its driver is not
 * bound yet, or another ERR_PTR.
 */
struct bcmring_vpm *bcmring_vpm_get(struct device_node *np)
{
	struct platform_device *pdev;
	struct bcmring_vpm *vpm;

	if (!of_match_node(bcmring_vpm_of_match, np))
		return ERR_PTR(-ENODEV);

	pdev = of_find_device_by_node(np);
	if (!pdev)
		return ERR_PTR(-EPROBE_DEFER);

	/* the device lock keeps an unbind from freeing @vpm under us */
	device_lock(&pdev->dev);
	vpm = pdev->dev.driver ? platform_get_drvdata(pdev) : NULL;
	if (!vpm) {
		device_unlock(&pdev->dev);
		put_device(&pdev->dev);
		return ERR_PTR(-EPROBE_DEFER);
	}

	/* the rproc device owns the memory @vpm lives in */
	get_device(&vpm->rproc->dev);
	device_unlock(&pdev->dev);
	put_device(&pdev->dev);

	return vpm;
}
EXPORT_SYMBOL_GPL(bcmring_vpm_get);

void bcmring_vpm_put(struct bcmring_vpm *vpm)
{
	put_device(&vpm->rproc->dev);
}
EXPORT_SYMBOL_GPL(bcmring_vpm_put);

/* ---- remoteproc ---------------------------------------------------- */

struct vpm_fw_record {
	__le32 addr;	/* ARM physical address */
	__le32 size;	/* payload bytes */
};

static bool bcmring_vpm_in_res(const struct resource *res, u32 addr, u32 size)
{
	return addr >= res->start && size <= resource_size(res) &&
	       addr - res->start <= resource_size(res) - size;
}

/*
 * The vendor loader accepts the two on-chip RAM base addresses without any
 * bounds check and DDR records that end below the family's limit.  Here
 * every record must lie inside one of the three regions the devicetree
 * gives.
 */
static int bcmring_vpm_record_ok(struct bcmring_vpm *vpm, u32 addr, u32 size)
{
	if (bcmring_vpm_in_res(&vpm->pram_res, addr, size) ||
	    bcmring_vpm_in_res(&vpm->dram_res, addr, size))
		return 0;
	if (bcmring_vpm_in_res(&vpm->ddr_res, addr, size) &&
	    (u64)addr + size <= vpm->family->ddr_limit)
		return 0;

	return -EINVAL;
}

/* The file is a bare sequence of records and ends after the last one. */
static int bcmring_vpm_sanity_check(struct rproc *rproc,
				    const struct firmware *fw)
{
	struct bcmring_vpm *vpm = rproc->priv;
	const struct vpm_fw_record *rec;
	size_t off = 0;
	u32 addr, size;

	if (!fw->size)
		return -EINVAL;

	while (off < fw->size) {
		if (fw->size - off < sizeof(*rec))
			goto bad;
		rec = (const void *)(fw->data + off);
		addr = get_unaligned_le32(&rec->addr);
		size = get_unaligned_le32(&rec->size);
		off += sizeof(*rec);
		if (size > fw->size - off)
			goto bad;
		if (bcmring_vpm_record_ok(vpm, addr, size)) {
			dev_err(vpm->dev,
				"firmware record for 0x%08x+0x%x is outside the DSP's memory\n",
				addr, size);
			return -EINVAL;
		}
		off += size;
	}

	return 0;

bad:
	dev_err(vpm->dev, "firmware image is truncated at offset %zu\n", off);
	return -EINVAL;
}

/*
 * Verified sequence (vpmHw_InitandHalt): halt, clock on, phase align, pulse
 * both reset-and-hold bits with the second reset word read and written back
 * after each change.  The core is then out of reset and held by the halt
 * word, with its clock running, which is the state the vendor loads in.
 */
static int bcmring_vpm_init_and_halt(struct bcmring_vpm *vpm)
{
	unsigned int val;
	int ret;

	mutex_lock(&vpm->power_lock);
	writel(1, vpm->vintc + VINTC_HALT);
	vpm->running = false;
	ret = bcmring_vpm_clock_on(vpm);
	if (ret) {
		regmap_clear_bits(vpm->chipc, CHIPC_VPM_CLK, VPM_CLK_ENABLE);
		goto unlock;
	}

	regmap_set_bits(vpm->chipc, CHIPC_SOFT_RESET1, VPM_RESET_HOLDS);
	regmap_read(vpm->chipc, CHIPC_SOFT_RESET2, &val);
	regmap_write(vpm->chipc, CHIPC_SOFT_RESET2, val);
	regmap_clear_bits(vpm->chipc, CHIPC_SOFT_RESET1, VPM_RESET_HOLDS);
	regmap_read(vpm->chipc, CHIPC_SOFT_RESET2, &val);
	regmap_write(vpm->chipc, CHIPC_SOFT_RESET2, val);
unlock:
	mutex_unlock(&vpm->power_lock);

	return ret;
}

/*
 * The remoteproc core's recovery path calls only stop, load, and start: no
 * sanity check, no prepare.  So this function does everything a load needs
 * by itself: it checks the image (which recovery reads from disk again),
 * resets the core, and leaves it halted with its clock on for start.
 */
static int bcmring_vpm_load(struct rproc *rproc, const struct firmware *fw)
{
	struct bcmring_vpm *vpm = rproc->priv;
	const struct vpm_fw_record *rec;
	void __iomem *va;
	size_t off = 0;
	u32 addr, size;
	int ret;

	ret = bcmring_vpm_sanity_check(rproc, fw);
	if (ret)
		return ret;

	ret = bcmring_vpm_init_and_halt(vpm);
	if (ret)
		return ret;

	while (off < fw->size) {
		rec = (const void *)(fw->data + off);
		addr = get_unaligned_le32(&rec->addr);
		size = get_unaligned_le32(&rec->size);
		off += sizeof(*rec);

		dev_dbg(vpm->dev, "load 0x%08x size %u\n", addr, size);
		if (!size) {
			continue;
		} else if (bcmring_vpm_in_res(&vpm->pram_res, addr, size)) {
			memcpy_toio(vpm->pram + (addr - vpm->pram_res.start),
				    fw->data + off, size);
		} else if (bcmring_vpm_in_res(&vpm->dram_res, addr, size)) {
			memcpy_toio(vpm->dram + (addr - vpm->dram_res.start),
				    fw->data + off, size);
		} else {
			/*
			 * DDR that is not Linux memory: an uncached mapping,
			 * so that nothing is left behind in the ARM's caches.
			 */
			va = ioremap(addr, size);
			if (!va) {
				bcmring_vpm_halt(vpm);
				return -ENOMEM;
			}
			memcpy_toio(va, fw->data + off, size);
			iounmap(va);
		}
		off += size;
	}

	return 0;
}

static void bcmring_vpm_transport_init(struct bcmring_vpm *vpm)
{
	unsigned int i;

	/* verified order: status word, magic, rings */
	writew(VPM_MSG_IDLE, vpm->vram + VRAM_STATUS);
	writew(VPM_MAGIC, vpm->vram + VRAM_MAGIC);
	for (i = 0; i < VPM_NUM_RINGS; i++)
		vpm_ring_init(&vpm->rings[i]);
}

static int bcmring_vpm_start(struct rproc *rproc)
{
	struct bcmring_vpm *vpm = rproc->priv;
	u16 before, val;
	int ret;

	before = readw(vpm->vram + VRAM_CMD);

	ret = bcmring_vpm_run(vpm);
	if (ret)
		return ret;

	/*
	 * The vendor lets the firmware run for 50 ms before it initializes
	 * the mailbox and the rings, so the firmware does not look at them
	 * before the first interrupt (inferred).  Keep that order.
	 */
	msleep(VPM_BOOT_SETTLE_MS);

	mutex_lock(&vpm->job_lock);
	bcmring_vpm_transport_init(vpm);

	/*
	 * There is no boot handshake.  The only sign of life is that the
	 * firmware writes "idle" to the command word (inferred: the host
	 * waits for that value before every command and never writes it).
	 * After a restart without a power cycle the word still holds the
	 * value from the previous run, and the check proves nothing.
	 */
	ret = readw_poll_timeout(vpm->vram + VRAM_CMD, val,
				 val == VPM_MSG_IDLE, 1000, VPM_IDLE_TIMEOUT_US);
	if (ret) {
		mutex_unlock(&vpm->job_lock);
		dev_err(vpm->dev,
			"DSP did not come up (command word 0x%04x, 0x%04x before start)\n",
			val, before);
		bcmring_vpm_halt(vpm);
		return ret;
	}
	if (before == VPM_MSG_IDLE)
		dev_info(vpm->dev,
			 "DSP started; command word was idle already, alive check inconclusive\n");
	else
		dev_info(vpm->dev,
			 "DSP started and alive (command word 0x%04x -> idle)\n",
			 before);

	vpm->jobs = 0;
	vpm->timeouts = 0;
	vpm->async_events = 0;
	vpm->stale_results = 0;
	vpm->failed = false;
	WRITE_ONCE(vpm->dead, false);
	vpm->overdue = false;
	vpm->ev = VPM_EV_NONE;
	vpm->online = true;
	/* armed under the lock, so that it cannot land on top of a job */
	bcmring_vpm_idle(vpm);
	mutex_unlock(&vpm->job_lock);

	enable_irq(vpm->irq);

	return 0;
}

/*
 * Halt and gate the clock, the state the vendor puts the core in between
 * jobs.  The vendor's shutdown path also sets bits 31:30 of the second
 * reset word ("unhold" requests in the old public headers) before gating
 * the clock.  GUESS: that write is left out, because its effect is not
 * known and the halted state is the one measured on idle phones; the next
 * start pulses the resets as a first start does.
 */
static int bcmring_vpm_stop(struct rproc *rproc)
{
	struct bcmring_vpm *vpm = rproc->priv;

	mutex_lock(&vpm->job_lock);
	if (vpm->online)
		disable_irq(vpm->irq);
	vpm->online = false;
	mutex_unlock(&vpm->job_lock);

	cancel_delayed_work_sync(&vpm->halt_work);
	cancel_work_sync(&vpm->assert_work);
	bcmring_vpm_halt(vpm);

	return 0;
}

static const struct rproc_ops bcmring_vpm_rproc_ops = {
	.start = bcmring_vpm_start,
	.stop = bcmring_vpm_stop,
	.load = bcmring_vpm_load,
	.sanity_check = bcmring_vpm_sanity_check,
};

/* ---- character device ---------------------------------------------- */

static struct bcmring_vpm *bcmring_vpm_from_file(struct file *file)
{
	return container_of(file->private_data, struct bcmring_vpm, misc);
}

static int bcmring_vpm_open(struct inode *inode, struct file *file)
{
	struct bcmring_vpm *vpm = bcmring_vpm_from_file(file);

	/* keeps the memory behind @vpm until the last close */
	get_device(&vpm->rproc->dev);

	return nonseekable_open(inode, file);
}

static int bcmring_vpm_release(struct inode *inode, struct file *file)
{
	bcmring_vpm_put(bcmring_vpm_from_file(file));

	return 0;
}

static long bcmring_vpm_ioctl_info(struct bcmring_vpm *vpm, void __user *argp)
{
	struct bcmring_vpm_info info = {
		.abi_version = BCMRING_VPM_ABI_VERSION,
		.family = vpm->family->id,
		.num_channels = vpm->family->channels,
		.max_body = bcmring_vpm_max_body(vpm),
	};

	mutex_lock(&vpm->job_lock);
	if (!vpm->online)
		info.state = BCMRING_VPM_STATE_OFFLINE;
	else if (vpm->failed)
		info.state = BCMRING_VPM_STATE_FAILED;
	else
		info.state = BCMRING_VPM_STATE_ONLINE;
	info.jobs = vpm->jobs;
	info.timeouts = vpm->timeouts;
	info.asserts = vpm->asserts;
	info.async_events = vpm->async_events;
	info.stale_results = vpm->stale_results;
	mutex_unlock(&vpm->job_lock);

	return copy_to_user(argp, &info, sizeof(info)) ? -EFAULT : 0;
}

static long bcmring_vpm_ioctl_job(struct bcmring_vpm *vpm, void __user *argp)
{
	struct bcmring_vpm_kjob kjob = { };
	struct bcmring_vpm_job job;
	void *body = NULL, *reply = NULL;
	size_t reply_max;
	long ret;

	if (copy_from_user(&job, argp, sizeof(job)))
		return -EFAULT;

	if (job.body_len > bcmring_vpm_max_body(vpm))
		return -EMSGSIZE;

	/* a result cannot be larger than a ring */
	reply_max = min_t(size_t, job.reply_max, 2 * vpm->family->ring_words);

	if (job.body_len) {
		body = memdup_user(u64_to_user_ptr(job.body), job.body_len);
		if (IS_ERR(body))
			return PTR_ERR(body);
	}
	if (reply_max) {
		reply = kmalloc(reply_max, GFP_KERNEL);
		if (!reply) {
			ret = -ENOMEM;
			goto out;
		}
	}

	kjob.channel = job.channel;
	kjob.client = job.client;
	kjob.instance = job.instance;
	kjob.opcode = job.opcode;
	kjob.codec = job.codec;
	kjob.arg1 = job.arg1;
	kjob.arg2 = job.arg2;
	kjob.body = body;
	kjob.body_len = job.body_len;
	kjob.reply = reply;
	kjob.reply_max = reply_max;
	kjob.timeout_ms = job.timeout_ms;

	ret = bcmring_vpm_job(vpm, &kjob);
	if (ret)
		goto out;

	job.arg1 = kjob.arg1;
	job.arg2 = kjob.arg2;
	job.reply_len_field = kjob.reply_len_field;
	job.reply_len = kjob.reply_len;

	if (reply_max &&
	    copy_to_user(u64_to_user_ptr(job.reply), reply,
			 min(kjob.reply_len, reply_max)))
		ret = -EFAULT;
	else if (copy_to_user(argp, &job, sizeof(job)))
		ret = -EFAULT;
out:
	kfree(reply);
	kfree(body);
	return ret;
}

static long bcmring_vpm_ioctl(struct file *file, unsigned int cmd,
			      unsigned long arg)
{
	struct bcmring_vpm *vpm = bcmring_vpm_from_file(file);
	void __user *argp = (void __user *)arg;

	switch (cmd) {
	case BCMRING_VPM_IOC_INFO:
		return bcmring_vpm_ioctl_info(vpm, argp);
	case BCMRING_VPM_IOC_JOB:
		return bcmring_vpm_ioctl_job(vpm, argp);
	default:
		return -ENOTTY;
	}
}

static const struct file_operations bcmring_vpm_fops = {
	.owner = THIS_MODULE,
	.open = bcmring_vpm_open,
	.release = bcmring_vpm_release,
	.unlocked_ioctl = bcmring_vpm_ioctl,
	.compat_ioctl = compat_ptr_ioctl,
};

/* ---- platform driver ----------------------------------------------- */

static void bcmring_vpm_release_dma(void *data)
{
	struct dma_chan *chan = data;

	/*
	 * The DSP is stopped by now, but it may have been halted in the
	 * middle of one of its own copies; dw_dmac insists on an idle
	 * channel when it is released.
	 */
	dmaengine_terminate_sync(chan);
	dma_release_channel(chan);
}

static void __iomem *bcmring_vpm_map(struct platform_device *pdev,
				     const char *name, struct resource *out)
{
	struct resource *res;
	void __iomem *base;

	res = platform_get_resource_byname(pdev, IORESOURCE_MEM, name);
	base = devm_ioremap_resource(&pdev->dev, res);
	if (!IS_ERR(base) && out)
		*out = *res;

	return base;
}

static int bcmring_vpm_probe(struct platform_device *pdev)
{
	const struct bcmring_vpm_family *family;
	struct device *dev = &pdev->dev;
	struct device_node *np = dev->of_node;
	const char *fw_name = VPM_FW_NAME;
	struct bcmring_vpm *vpm;
	struct resource vram_res;
	struct rproc *rproc;
	struct clk *clk;
	unsigned int i;
	int ret;

	family = device_get_match_data(dev);
	if (!family)
		return -ENODEV;

	of_property_read_string(np, "firmware-name", &fw_name);

	rproc = devm_rproc_alloc(dev, dev_name(dev), &bcmring_vpm_rproc_ops,
				 fw_name, sizeof(*vpm));
	if (!rproc)
		return -ENOMEM;

	/*
	 * The firmware has to be put in place by the user first, and after
	 * an assertion the cause wants looking at: no automatic start and no
	 * automatic restart.
	 */
	rproc->auto_boot = false;
	rproc->recovery_disabled = true;

	vpm = rproc->priv;
	vpm->dev = dev;
	vpm->rproc = rproc;
	vpm->family = family;
	mutex_init(&vpm->power_lock);
	mutex_init(&vpm->job_lock);
	init_completion(&vpm->event);
	INIT_DELAYED_WORK(&vpm->halt_work, bcmring_vpm_halt_work);
	INIT_WORK(&vpm->assert_work, bcmring_vpm_assert_work);

	vpm->pram = bcmring_vpm_map(pdev, "pram", &vpm->pram_res);
	if (IS_ERR(vpm->pram))
		return PTR_ERR(vpm->pram);
	vpm->dram = bcmring_vpm_map(pdev, "dram", &vpm->dram_res);
	if (IS_ERR(vpm->dram))
		return PTR_ERR(vpm->dram);
	vpm->vram = bcmring_vpm_map(pdev, "vram", &vram_res);
	if (IS_ERR(vpm->vram))
		return PTR_ERR(vpm->vram);
	vpm->vintc = bcmring_vpm_map(pdev, "vintc", NULL);
	if (IS_ERR(vpm->vintc))
		return PTR_ERR(vpm->vintc);

	if (resource_size(&vram_res) <
	    VRAM_RINGS + VPM_NUM_RINGS * 2 * family->ring_words)
		return dev_err_probe(dev, -EINVAL, "shared RAM too small\n");

	for (i = 0; i < VPM_NUM_RINGS; i++) {
		vpm->rings[i].base = vpm->vram + VRAM_RINGS +
				     i * 2 * family->ring_words;
		vpm->rings[i].words = family->ring_words;
		vpm->rings[i].old_family =
			family->id == BCMRING_VPM_FAMILY_OLD;
	}

	ret = of_reserved_mem_region_to_resource(np, 0, &vpm->ddr_res);
	if (ret)
		return dev_err_probe(dev, ret, "no memory-region\n");

	vpm->chipc = syscon_regmap_lookup_by_phandle(np, "brcm,chipc");
	if (IS_ERR(vpm->chipc))
		return dev_err_probe(dev, PTR_ERR(vpm->chipc),
				     "no chip-control regmap\n");

	/* bus-interface gate of the shared RAM */
	clk = devm_clk_get_enabled(dev, "vram");
	if (IS_ERR(clk))
		return dev_err_probe(dev, PTR_ERR(clk), "no vram clock\n");

	vpm->irq = platform_get_irq(pdev, 0);
	if (vpm->irq < 0)
		return vpm->irq;
	ret = devm_request_irq(dev, vpm->irq, bcmring_vpm_irq, IRQF_NO_AUTOEN,
			       dev_name(dev), vpm);
	if (ret)
		return ret;

	/* see the comment at the top of the file */
	if (of_property_present(np, "dmas")) {
		vpm->dma = dma_request_chan(dev, "vpm");
		if (IS_ERR(vpm->dma))
			return dev_err_probe(dev, PTR_ERR(vpm->dma),
					     "cannot hold the DSP's DMA channel\n");
		ret = devm_add_action_or_reset(dev, bcmring_vpm_release_dma,
					       vpm->dma);
		if (ret)
			return ret;
	} else {
		dev_warn(dev,
			 "no dmas: the DMA controller may be switched off under the DSP\n");
	}

	platform_set_drvdata(pdev, vpm);

	ret = devm_rproc_add(dev, rproc);
	if (ret)
		return ret;

	vpm->misc.minor = MISC_DYNAMIC_MINOR;
	vpm->misc.name = "bcmring-vpm";
	vpm->misc.fops = &bcmring_vpm_fops;
	vpm->misc.parent = dev;

	return misc_register(&vpm->misc);
}

static void bcmring_vpm_remove(struct platform_device *pdev)
{
	struct bcmring_vpm *vpm = platform_get_drvdata(pdev);

	misc_deregister(&vpm->misc);

	/*
	 * rproc_del() follows from devm and normally stops the DSP, but it
	 * gives up silently when its lock wait is interrupted.  Stop here, so
	 * that nothing touches the mappings once they are gone.
	 */
	bcmring_vpm_stop(vpm->rproc);
}

static struct platform_driver bcmring_vpm_driver = {
	.probe = bcmring_vpm_probe,
	.remove = bcmring_vpm_remove,
	.driver = {
		.name = "bcmring-vpm",
		.of_match_table = bcmring_vpm_of_match,
	},
};
module_platform_driver(bcmring_vpm_driver);

MODULE_FIRMWARE(VPM_FW_NAME);
MODULE_DESCRIPTION("Broadcom BCMRING voice DSP (VPM) remoteproc and job interface");
MODULE_LICENSE("GPL");
