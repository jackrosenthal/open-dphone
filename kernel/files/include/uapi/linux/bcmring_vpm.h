/* SPDX-License-Identifier: GPL-2.0-only WITH Linux-syscall-note */
/*
 * Broadcom BCMRING voice DSP ("VPM") job interface, /dev/bcmring-vpm.
 *
 * The DSP is a compute coprocessor: the host hands it one "job" at a time
 * (a codec frame, an echo-canceller frame, a band split, a configuration
 * block) and gets the result back.  One ioctl carries one job.
 */
#ifndef _UAPI_LINUX_BCMRING_VPM_H
#define _UAPI_LINUX_BCMRING_VPM_H

#include <linux/ioctl.h>
#include <linux/types.h>

#define BCMRING_VPM_ABI_VERSION		1

/* job clients */
#define BCMRING_VPM_CLIENT_PVE		1	/* vocoders */
#define BCMRING_VPM_CLIENT_SBAEC	3	/* sub-band acoustic echo canceller */
#define BCMRING_VPM_CLIENT_HEC		4	/* handset/headset echo canceller */
#define BCMRING_VPM_CLIENT_BAND		8	/* band split/combine; new family only */

/* firmware/SoC families; they differ in ring size, channels, and clients */
#define BCMRING_VPM_FAMILY_OLD		0	/* BCM11109, BCM11170 */
#define BCMRING_VPM_FAMILY_NEW		1	/* BCM11188, BCM11189 */

#define BCMRING_VPM_STATE_OFFLINE	0	/* not started */
#define BCMRING_VPM_STATE_ONLINE	1	/* accepting jobs */
#define BCMRING_VPM_STATE_FAILED	2	/* DSP assertion or protocol error */

/**
 * struct bcmring_vpm_info - driver and DSP state
 * @abi_version: BCMRING_VPM_ABI_VERSION of the running driver
 * @family: BCMRING_VPM_FAMILY_*
 * @state: BCMRING_VPM_STATE_*
 * @num_channels: channel numbers 0..@num_channels - 1 are accepted
 * @max_body: largest request body in bytes
 * @jobs: jobs completed since the DSP was started
 * @timeouts: jobs that got no result in time
 * @asserts: DSP assertion reports
 * @async_events: asynchronous events seen (they are not served)
 * @stale_results: results dropped because no job was waiting for them
 * @reserved: zero
 */
struct bcmring_vpm_info {
	__u32 abi_version;
	__u32 family;
	__u32 state;
	__u32 num_channels;
	__u32 max_body;
	__u32 jobs;
	__u32 timeouts;
	__u32 asserts;
	__u32 async_events;
	__u32 stale_results;
	__u32 reserved[6];
};

/**
 * struct bcmring_vpm_job - one synchronous job
 * @channel: channel number, chosen by the caller.  The DSP learns of a
 *	channel from the first job that names it; the reply is matched on it.
 * @client: BCMRING_VPM_CLIENT_*
 * @instance: instance of the client (voice stream, canceller, filter bank)
 * @opcode: operation, client specific
 * @codec: codec number, PVE only (0 otherwise); numbering is per family
 * @arg1: first argument; on return the result's first value
 * @arg2: second argument; on return the result's second value
 * @reply_len_field: on return, the body-length word of the result header.
 *	The DSP leaves 0 here for some results that carry a body, so use
 *	@reply_len for sizing and this only where a client documents it
 *	(e.g., the byte count of an encoded frame).
 * @body_len: request body in bytes
 * @reply_max: size of the buffer at @reply
 * @reply_len: on return, the result body in bytes as carried by the
 *	transport (always even; it may exceed @reply_max, in which case only
 *	@reply_max bytes were stored)
 * @timeout_ms: how long to wait for the result; 0 selects 1000 ms
 * @body: pointer to the request body (may be 0 if @body_len is 0)
 * @reply: pointer to the buffer for the result body (may be 0 if
 *	@reply_max is 0)
 *
 * Errors: ENODEV the DSP is not started, EIO the DSP reported an assertion
 * or the transport lost synchronization (restart the DSP), ETIMEDOUT no
 * result arrived, EBUSY an earlier job timed out and its result is still
 * outstanding (retry later or restart the DSP), EPROTO the result does not
 * belong to the request, EINVAL bad channel or client, EMSGSIZE the body is
 * too long.
 */
struct bcmring_vpm_job {
	__u16 channel;
	__u16 client;
	__u16 instance;
	__u16 opcode;
	__u16 codec;
	__u16 arg1;
	__u16 arg2;
	__u16 reply_len_field;
	__u32 body_len;
	__u32 reply_max;
	__u32 reply_len;
	__u32 timeout_ms;
	__u64 body;
	__u64 reply;
};

#define BCMRING_VPM_IOC_MAGIC	0xb9

#define BCMRING_VPM_IOC_INFO	_IOR(BCMRING_VPM_IOC_MAGIC, 0, struct bcmring_vpm_info)
#define BCMRING_VPM_IOC_JOB	_IOWR(BCMRING_VPM_IOC_MAGIC, 1, struct bcmring_vpm_job)

#endif /* _UAPI_LINUX_BCMRING_VPM_H */
