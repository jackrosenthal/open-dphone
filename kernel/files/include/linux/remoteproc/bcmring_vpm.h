/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Broadcom BCMRING voice DSP ("VPM"): in-kernel job interface.
 */
#ifndef _LINUX_REMOTEPROC_BCMRING_VPM_H
#define _LINUX_REMOTEPROC_BCMRING_VPM_H

#include <linux/err.h>
#include <linux/types.h>

struct bcmring_vpm;
struct device_node;

/**
 * struct bcmring_vpm_kjob - one synchronous DSP job
 * @channel: channel number (the caller's choice, below the family's limit)
 * @client: job client (BCMRING_VPM_CLIENT_* of the UAPI header)
 * @instance: instance of the client
 * @opcode: operation
 * @codec: codec number, vocoder client only
 * @arg1: first argument; replaced by the result's first value
 * @arg2: second argument; replaced by the result's second value
 * @reply_len_field: out: the body-length word of the result header
 * @body: request body, @body_len bytes
 * @body_len: request body length in bytes
 * @reply: buffer for the result body, @reply_max bytes
 * @reply_max: size of @reply
 * @reply_len: out: result body in bytes as carried by the transport; if it
 *	exceeds @reply_max only @reply_max bytes were copied
 * @timeout_ms: time to wait for the result; 0 selects the default
 */
struct bcmring_vpm_kjob {
	u16 channel;
	u16 client;
	u16 instance;
	u16 opcode;
	u16 codec;
	u16 arg1;
	u16 arg2;
	u16 reply_len_field;
	const void *body;
	size_t body_len;
	void *reply;
	size_t reply_max;
	size_t reply_len;
	unsigned int timeout_ms;
};

#if IS_ENABLED(CONFIG_BCMRING_VPM_REMOTEPROC)

struct bcmring_vpm *bcmring_vpm_get(struct device_node *np);
void bcmring_vpm_put(struct bcmring_vpm *vpm);
int bcmring_vpm_job(struct bcmring_vpm *vpm, struct bcmring_vpm_kjob *job);

#else

static inline struct bcmring_vpm *bcmring_vpm_get(struct device_node *np)
{
	return ERR_PTR(-ENODEV);
}

static inline void bcmring_vpm_put(struct bcmring_vpm *vpm)
{
}

static inline int bcmring_vpm_job(struct bcmring_vpm *vpm,
				  struct bcmring_vpm_kjob *job)
{
	return -ENODEV;
}

#endif

#endif /* _LINUX_REMOTEPROC_BCMRING_VPM_H */
