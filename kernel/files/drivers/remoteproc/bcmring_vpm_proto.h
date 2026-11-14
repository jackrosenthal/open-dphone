/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Broadcom BCMRING voice DSP ("VPM"): wire format of the job transport.
 *
 * Ring rules and job headers, as pure functions on a shared-memory window.
 * They are kept apart from the driver so that a host-side self-test can
 * compile this very file against plain memory.  The includer provides the
 * fixed-width types (u16, ...), bool, the errno values, and four accessors:
 *
 *	vpm_rd16(p)			read a little-endian 16-bit word
 *	vpm_wr16(v, p)			write one
 *	vpm_copy_in(p, src, len)	copy bytes into the window
 *	vpm_copy_out(dst, p, len)	copy bytes out of the window
 *
 * DRAFT.  Never run against a DSP.  Every rule below was recovered by static
 * analysis of the vendor's host-side code for both SoC families and agrees
 * with ring snapshots of running stock phones; how the DSP firmware itself
 * treats the rings is inferred from that host code only.
 *
 * A ring of N 16-bit words:
 *
 *	word 0		write descriptor, owned by the producer
 *	word 1		read descriptor, owned by the consumer
 *	words 2..N-1	data area, B = N - 2 words
 *
 * A descriptor is (wrap << 15) | index, index in 0..B-1.  Equal descriptors
 * mean "empty"; equal indexes with different wrap bits mean "full".  A frame
 * is one length word L (bit 15 clear) followed by L payload words and is
 * never split.  A word with bit 15 set in place of a length word is a skip
 * marker: the next frame is at index 0.  The initial fill value 0xab00 is
 * such a marker.
 *
 * The two SoC families place the markers differently, and each family's
 * consumer matches its producer:
 *
 *  - New family (BCM11188/11189).  The producer terminates every frame: if
 *    more than VPM_RING_NEW_TAIL words remain behind the frame it writes a 0
 *    there, otherwise it writes the skip marker and returns to index 0 at
 *    once.  The consumer looks at the word behind the frame it has just
 *    taken.  A frame therefore never starts with fewer than
 *    VPM_RING_NEW_TAIL + 1 words to the end of the data area.
 *
 *  - Old family (BCM11109/11170).  The producer writes nothing behind a
 *    frame.  Before writing it checks whether the frame fits up to the end
 *    of the data area and, if not, writes the skip marker at its index and
 *    starts at index 0.  The consumer tests the word under its index for the
 *    marker before reading it as a length.
 *
 * Both normalize "index == B" to index 0 with the wrap bit toggled.  A
 * consumer publishes its descriptor only when it frees the frame.
 */
#ifndef BCMRING_VPM_PROTO_H
#define BCMRING_VPM_PROTO_H

#define VPM_RING_FILL		0xab00
#define VPM_RING_WRAP		0x8000
#define VPM_RING_INDEX		0x7fff
/* largest frame of the new family, and the tail kept free for it */
#define VPM_RING_NEW_TAIL	0x1067

/* job clients */
#define VPM_CLIENT_PVE		1	/* vocoders */
#define VPM_CLIENT_SBAEC	3	/* sub-band acoustic echo canceller */
#define VPM_CLIENT_HEC		4	/* handset/headset echo canceller */
#define VPM_CLIENT_BAND		8	/* band split/combine, new family only */

#define VPM_HDR_MAX_WORDS	10

struct vpm_ring {
	void __iomem *base;	/* word 0 of the ring */
	u16 words;		/* N */
	u16 own;		/* our descriptor as last published */
	u16 next;		/* consumer: descriptor to publish on free */
	bool old_family;
};

static inline void __iomem *vpm_ring_data(const struct vpm_ring *ring,
					  unsigned int index)
{
	return ring->base + 4 + 2 * index;
}

/*
 * The other side updates its descriptor without any lock: read it until two
 * consecutive reads agree, as the vendor does.  Bounded so that broken
 * memory cannot hang the caller; the last value read is used then.
 */
static inline u16 vpm_ring_peer(void __iomem *desc)
{
	unsigned int tries = 64;
	u16 a, b;

	do {
		a = vpm_rd16(desc);
		b = vpm_rd16(desc);
	} while (a != b && --tries);

	return b;
}

/* Host-side initialization: both descriptors 0, data area filled. */
static inline void vpm_ring_init(struct vpm_ring *ring)
{
	unsigned int i;

	vpm_wr16(0, ring->base);
	vpm_wr16(0, ring->base + 2);
	for (i = 0; i < ring->words - 2U; i++)
		vpm_wr16(VPM_RING_FILL, vpm_ring_data(ring, i));
	ring->own = 0;
	ring->next = 0;
}

/*
 * Largest payload (in words) that vpm_ring_put() accepts on an empty ring
 * whatever the position of the indexes.  New family: the tail rule
 * guarantees VPM_RING_NEW_TAIL.  Old family: a frame goes either behind the
 * write index or at index 0 in front of the read index, so only half of the
 * data area is guaranteed.
 */
static inline unsigned int vpm_ring_max_payload(const struct vpm_ring *ring)
{
	unsigned int b = ring->words - 2;

	if (ring->old_family)
		return b / 2 - 1;

	return VPM_RING_NEW_TAIL;
}

/* Producer side: true if the consumer has freed everything written. */
static inline bool vpm_ring_drained(const struct vpm_ring *ring)
{
	return vpm_ring_peer(ring->base + 2) == ring->own;
}

/*
 * Producer: write one frame made of @hdr_words header words followed by
 * @body_len bytes (an odd byte count is padded with a zero byte), then
 * publish the write descriptor.
 *
 * Unlike the vendor code, which only tests "ring full", this checks that the
 * frame (and, on the new family, the terminating word) does not reach the
 * consumer's index.
 *
 * Returns 0, -EMSGSIZE if the frame can never fit, or -ENOSPC if it does not
 * fit now.
 */
static inline int vpm_ring_put(struct vpm_ring *ring, const u16 *hdr,
			       unsigned int hdr_words, const void *body,
			       unsigned int body_len)
{
	unsigned int b = ring->words - 2;
	unsigned int body_words = (body_len + 1) / 2;
	unsigned int len = hdr_words + body_words;
	unsigned int total = len + 1;
	u16 rd = vpm_ring_peer(ring->base + 2);
	u16 wr = ring->own;
	unsigned int r = rd & VPM_RING_INDEX;
	unsigned int w = wr & VPM_RING_INDEX;
	u16 wrap = wr & VPM_RING_WRAP;
	void __iomem *p;
	unsigned int i, rem;

	if (!len || len > vpm_ring_max_payload(ring))
		return -EMSGSIZE;

	/* full */
	if (r == w && rd != wr)
		return -ENOSPC;

	if (ring->old_family) {
		if (r > w) {
			if (r - w < total)
				return -ENOSPC;
		} else if (b - w < total) {
			if (r < total)
				return -ENOSPC;
			vpm_wr16(VPM_RING_WRAP | (b - w - 1),
				 vpm_ring_data(ring, w));
			w = 0;
			wrap ^= VPM_RING_WRAP;
		}
	} else {
		/* the frame, and the terminator unless it ends at B */
		if (r > w ? r - w <= total : b - w < total)
			return -ENOSPC;
	}

	p = vpm_ring_data(ring, w);
	vpm_wr16(len, p);
	p += 2;
	for (i = 0; i < hdr_words; i++, p += 2)
		vpm_wr16(hdr[i], p);
	if (body_len & ~1U)
		vpm_copy_in(p, body, body_len & ~1U);
	if (body_len & 1)
		vpm_wr16(((const u8 *)body)[body_len - 1],
			 p + (body_len & ~1U));

	w += total;
	rem = b - w;
	if (!ring->old_family && rem) {
		if (rem > VPM_RING_NEW_TAIL) {
			vpm_wr16(0, vpm_ring_data(ring, w));
		} else {
			vpm_wr16(VPM_RING_WRAP | (rem - 1),
				 vpm_ring_data(ring, w));
			w = 0;
			wrap ^= VPM_RING_WRAP;
		}
	}
	if (w == b) {
		w = 0;
		wrap ^= VPM_RING_WRAP;
	}

	ring->own = wrap | w;
	vpm_wr16(ring->own, ring->base);

	return 0;
}

/*
 * Consumer: look at the next frame without freeing it.  Returns its payload
 * length in words and sets @payload, 0 if the ring is empty, or -EPROTO if
 * what is under the read index cannot be a frame.
 */
static inline int vpm_ring_peek(struct vpm_ring *ring, void __iomem **payload)
{
	unsigned int b = ring->words - 2;
	u16 wr = vpm_ring_peer(ring->base);
	u16 rd = ring->own;
	unsigned int i = rd & VPM_RING_INDEX;
	u16 wrap = rd & VPM_RING_WRAP;
	unsigned int len, n;

	if (wr == rd)
		return 0;
	if (i >= b)
		return -EPROTO;

	if (ring->old_family &&
	    (vpm_rd16(vpm_ring_data(ring, i)) & VPM_RING_WRAP)) {
		i = 0;
		wrap ^= VPM_RING_WRAP;
	}

	len = vpm_rd16(vpm_ring_data(ring, i));
	if (!len || (len & VPM_RING_WRAP) || i + 1 + len > b)
		return -EPROTO;

	*payload = vpm_ring_data(ring, i + 1);

	n = i + 1 + len;
	if (!ring->old_family && n < b &&
	    (vpm_rd16(vpm_ring_data(ring, n)) & VPM_RING_WRAP))
		n = b;
	if (n == b) {
		n = 0;
		wrap ^= VPM_RING_WRAP;
	}
	ring->next = wrap | n;

	return len;
}

/* Consumer: free the frame returned by the last vpm_ring_peek(). */
static inline void vpm_ring_free(struct vpm_ring *ring)
{
	ring->own = ring->next;
	vpm_wr16(ring->own, ring->base + 2);
}

/*
 * Job header.  Common part (words): 0 job type (always 0), 1 channel,
 * 2 client, 3 not written by the vendor host (stack residue there; 0 here),
 * 4 instance.  Then per client:
 *
 *	PVE	5 codec, 6 opcode, 7 arg1, 8 arg2, 9 body length	(10 words)
 *	SBAEC	5 opcode, 6 arg1, 7 arg2, 8 body length, 9 unused	(10 words)
 *	HEC	5 opcode, 6 arg1, 7 arg2, 8 body length			(9 words)
 *	BAND	as HEC							(9 words)
 *
 * The body length is in bytes.  A result repeats the layout with its own
 * values in arg1, arg2, and the length.  The DSP has been seen to leave 0 in
 * the length word of results that do carry a body, so the frame length is
 * what tells how much body there is.
 */
struct vpm_job_hdr {
	u16 channel;
	u16 client;
	u16 instance;
	u16 codec;	/* PVE only */
	u16 opcode;
	u16 arg1;
	u16 arg2;
	u16 len;	/* the header's body-length word */
};

static inline unsigned int vpm_hdr_words(unsigned int client)
{
	switch (client) {
	case VPM_CLIENT_PVE:
	case VPM_CLIENT_SBAEC:
		return 10;
	case VPM_CLIENT_HEC:
	case VPM_CLIENT_BAND:
		return 9;
	default:
		return 0;
	}
}

/* Returns the number of header words written to @w, 0 for an unknown client. */
static inline unsigned int vpm_hdr_encode(const struct vpm_job_hdr *h, u16 *w)
{
	unsigned int n = vpm_hdr_words(h->client);
	unsigned int i = 5;

	if (!n)
		return 0;

	w[0] = 0;
	w[1] = h->channel;
	w[2] = h->client;
	w[3] = 0;
	w[4] = h->instance;
	if (h->client == VPM_CLIENT_PVE)
		w[i++] = h->codec;
	w[i++] = h->opcode;
	w[i++] = h->arg1;
	w[i++] = h->arg2;
	w[i++] = h->len;
	if (h->client == VPM_CLIENT_SBAEC)
		w[i++] = 0;

	return n;
}

/*
 * Returns the number of header words, or -EPROTO if @w (@nwords words) is
 * not a job frame of a known client.
 */
static inline int vpm_hdr_decode(const u16 *w, unsigned int nwords,
				 struct vpm_job_hdr *h)
{
	unsigned int n, i = 5;

	if (nwords < 5)
		return -EPROTO;
	n = vpm_hdr_words(w[2]);
	if (!n || nwords < n)
		return -EPROTO;

	h->channel = w[1];
	h->client = w[2];
	h->instance = w[4];
	h->codec = h->client == VPM_CLIENT_PVE ? w[i++] : 0;
	h->opcode = w[i++];
	h->arg1 = w[i++];
	h->arg2 = w[i++];
	h->len = w[i++];

	return n;
}

#endif /* BCMRING_VPM_PROTO_H */
