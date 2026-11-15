/* SPDX-License-Identifier: GPL-2.0-only */
/*
 * Debugging macro include: append the low-level debug output to a one-page
 * text ring in on-chip RAM that survives a reset (Broadcom BCMRING ARAM).
 *
 * Selected by CONFIG_DEBUG_BCMRING_RAMLOG.
 *
 * Page layout, little-endian words:
 *   0x000  magic "RBLG" (0x474c4252), written by whoever initializes the
 *          page (the RAM-boot loader); never written here
 *   0x004  offset of the next character, 16..4095
 *   0x008  number of characters written since the page was initialized
 *   0x00c  the character being written (scratch)
 *   0x010  text, a ring of 4080 characters
 *
 * The page address must be page aligned, physically and virtually: the code
 * gets the page base back from a character address by clearing the low
 * twelve bits, which lets it work with the two registers the callers
 * provide.  A next-character offset that is out of range (e.g., because
 * nobody initialized the page) is forced into range, so the worst a stale
 * page can do is produce an unreadable log.
 */

		.macro	addruart, rp, rv, tmp
		ldr	\rp, =CONFIG_DEBUG_BCMRING_RAMLOG_PHYS
		ldr	\rv, =CONFIG_DEBUG_BCMRING_RAMLOG_VIRT
		.endm

		.macro	bcmring_ramlog_pos, rd, rx
		ldr	\rd, [\rx, #4]
		mov	\rd, \rd, lsl #20
		mov	\rd, \rd, lsr #20
		cmp	\rd, #16
		movlo	\rd, #16
		.endm

		.macro	senduart, rd, rx
		strb	\rd, [\rx, #12]
		.endm

		.macro	busyuart, rd, rx
		bcmring_ramlog_pos \rd, \rx
		add	\rx, \rx, \rd		@ address of the next character
		sub	\rd, \rx, \rd		@ page base
		ldrb	\rd, [\rd, #12]
		strb	\rd, [\rx]
		bic	\rx, \rx, #0x0f00
		bic	\rx, \rx, #0x00ff	@ page base again
		bcmring_ramlog_pos \rd, \rx
		add	\rd, \rd, #1
		cmp	\rd, #4096
		movhs	\rd, #16
		str	\rd, [\rx, #4]
		ldr	\rd, [\rx, #8]
		add	\rd, \rd, #1
		str	\rd, [\rx, #8]
		.endm

		.macro	waituartcts, rd, rx
		.endm

		.macro	waituarttxrdy, rd, rx
		.endm
