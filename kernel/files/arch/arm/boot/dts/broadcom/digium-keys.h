/* SPDX-License-Identifier: (GPL-2.0-only OR BSD-2-Clause) */
/*
 * Key codes for the Digium/Sangoma D40/D45/D50/D70/D60/D62/D65 front panel,
 * plus the pin-mux helper for the BCMRING chip-control mux registers.
 *
 * The vendor kernels report every key as EV_KEY with a code equal to an
 * ASCII character (the Broadcom "OnePhone" convention; the legend is in
 * the help text of the vendor /bin/input tool, but it does not describe
 * these phones reliably: its entries for 'b', 'c', 'e', 'o', and 'v' do not
 * match what the keys turned out to be).  For example the vendor hookswitch
 * code 75 is 'K', not KEY_KP4, and the kernel's d-pad lockout compares
 * against 'U', 'D', 'L', and 'R'.
 *
 * Two mappings are provided:
 *
 *   default                 Every key keeps its vendor ASCII code.  This is
 *                           lossless and what the vendor userspace saw.
 *
 *   DIGIUM_KEYS_PROPOSED    Standard Linux codes where the key's function
 *                           is known and Linux has a code for it.
 *                           Everything else becomes KEY_MACROn as a
 *                           placeholder.  The vendor codes cannot be kept
 *                           next to standard ones because they collide
 *                           numerically (e.g., 'g' == KEY_UP,
 *                           'i' == KEY_LEFT, 'j' == KEY_RIGHT,
 *                           'r' == KEY_VOLUMEDOWN).
 *
 * Confirmed by pressing keys on a D60 and a D40: digits 1, 2,
 * 3, and '#', the hookswitch, volume, the four directions, SOFT1..SOFT4
 * (left to right under the screen), LINE1 (top) and LINE2 beside the screen,
 * MUTE, SPEAKER, HEADSET, HOLD, TRANSFER, REDIAL, MSGS (voicemail), SELECT
 * (check mark), and CANCEL (X).  The hookswitch reports release when the
 * handset is lifted and press when it is replaced.  Neither phone has any
 * other key.  Confirmed the same way on a D50: LINE1..LINE4 (top to bottom),
 * CONF, INFO, and BLF1..BLF10 (one column, top to bottom).  LINE4 is at
 * this position on the D50 only; the D6x tables place the same character
 * elsewhere in the matrix.  This accounts for every code in the D40/D45/D50
 * table.
 *
 * Identified from the vendor phone application, not by pressing: the eight
 * characters that occur only in the D6x and D70 tables ('G', 'I', 'J', 'M',
 * 'N', 'O', 'P', and 'Q').  /app/bin/phone turns each key code into a named
 * event in one switch statement, which is the same in the D60, D62, D65,
 * and D70 images (2_9_26 and 2_9_28): 'G' "fn.menu", 'I' "fn.status",
 * 'J' "fn.apps", 'M' "blf.left", 'N' "blf.right", 'O' "line.4.down",
 * 'P' "line.5.down" (lines count from 0), and 'Q' "fn.dir".  The same
 * switch gives every key that was pressed the function found by pressing
 * it.
 */
#ifndef DIGIUM_KEYS_H
#define DIGIUM_KEYS_H

#include <dt-bindings/input/input.h>

/*
 * One pinctrl-single "bits" cell triple for GPIO pad <pin> and mux function
 * <func>, relative to chip-control offset 0x118 (GpioMux_0_7).  Each
 * register holds eight 4-bit fields.
 */
#define BCMRING_MUX_REG(pin)		(((pin) / 8) * 4)
#define BCMRING_MUX_VAL(pin, func)	((func) << (((pin) % 8) * 4))
#define BCMRING_MUX_MASK(pin)		(0xf << (((pin) % 8) * 4))

/* Mux functions (old mainline chipcHw_reg.h; 0, 1, 4, 5, 6, 13 verified). */
#define BCMRING_MUX_GPIO	0
#define BCMRING_MUX_KEYPAD	1
#define BCMRING_MUX_I2CH	2
#define BCMRING_MUX_SPI		3
#define BCMRING_MUX_UART	4
#define BCMRING_MUX_LEDMTXP	5
#define BCMRING_MUX_LEDMTXS	6
#define BCMRING_MUX_SDIO0	7
#define BCMRING_MUX_SDIO1	8
#define BCMRING_MUX_PCM		9
#define BCMRING_MUX_I2S		10
#define BCMRING_MUX_ETM		11
#define BCMRING_MUX_DEBUG	12
#define BCMRING_MUX_MISC	13

#ifndef DIGIUM_KEYS_PROPOSED

/* Vendor ASCII codes. */
#define DGK_0		48	/* '0' */
#define DGK_1		49
#define DGK_2		50
#define DGK_3		51
#define DGK_4		52
#define DGK_5		53
#define DGK_6		54
#define DGK_7		55
#define DGK_8		56
#define DGK_9		57
#define DGK_STAR	42	/* '*' */
#define DGK_POUND	35	/* '#' */
#define DGK_UP		85	/* 'U' */
#define DGK_DOWN	68	/* 'D' */
#define DGK_LEFT	76	/* 'L' */
#define DGK_RIGHT	82	/* 'R' */
#define DGK_SELECT	67	/* 'C', check mark */
#define DGK_CANCEL	98	/* 'b', X */
#define DGK_VOLDOWN	60	/* '<' */
#define DGK_VOLUP	62	/* '>' */
#define DGK_SOFT1	40	/* '(' */
#define DGK_SOFT2	99	/* 'c' */
#define DGK_SOFT3	101	/* 'e' */
#define DGK_SOFT4	41	/* ')' */
#define DGK_HOOK	75	/* 'K' */
#define DGK_MUTE	109	/* 'm' */
#define DGK_SPEAKER	102	/* 'f' */
#define DGK_HEADSET	103	/* 'g' */
#define DGK_HOLD	104	/* 'h' */
#define DGK_TRANSFER	116	/* 't' */
#define DGK_REDIAL	100	/* 'd' */
#define DGK_MSGS	97	/* 'a' */
#define DGK_CONF	114	/* 'r' */
#define DGK_INFO	113	/* 'q' */
#define DGK_LINE1	106	/* 'j' */
#define DGK_LINE2	105	/* 'i' */
#define DGK_LINE3	117	/* 'u' */
#define DGK_LINE4	107	/* 'k' */
#define DGK_BLF1	119	/* 'w' */
#define DGK_BLF2	120	/* 'x' */
#define DGK_BLF3	121	/* 'y' */
#define DGK_BLF4	122	/* 'z' */
#define DGK_BLF5	65	/* 'A' */
#define DGK_BLF6	70	/* 'F' */
#define DGK_BLF7	118	/* 'v' */
#define DGK_BLF8	110	/* 'n' */
#define DGK_BLF9	111	/* 'o' */
#define DGK_BLF10	112	/* 'p' */
#define DGK_MENU	71	/* 'G' */
#define DGK_STATUS	73	/* 'I' */
#define DGK_APPS	74	/* 'J' */
#define DGK_BLF_LEFT	77	/* 'M', previous page of the BLF keys */
#define DGK_BLF_RIGHT	78	/* 'N', next page of the BLF keys */
#define DGK_LINE5	79	/* 'O' */
#define DGK_LINE6	80	/* 'P' */
#define DGK_CONTACTS	81	/* 'Q', directory */

#else /* DIGIUM_KEYS_PROPOSED */

/* Self-evident from the character. */
#define DGK_0		KEY_NUMERIC_0
#define DGK_1		KEY_NUMERIC_1
#define DGK_2		KEY_NUMERIC_2
#define DGK_3		KEY_NUMERIC_3
#define DGK_4		KEY_NUMERIC_4
#define DGK_5		KEY_NUMERIC_5
#define DGK_6		KEY_NUMERIC_6
#define DGK_7		KEY_NUMERIC_7
#define DGK_8		KEY_NUMERIC_8
#define DGK_9		KEY_NUMERIC_9
#define DGK_STAR	KEY_NUMERIC_STAR
#define DGK_POUND	KEY_NUMERIC_POUND
/*
 * Function confirmed by pressing the key (see the top of this file), and
 * Linux has a code for it.
 */
#define DGK_UP		KEY_UP
#define DGK_DOWN	KEY_DOWN
#define DGK_LEFT	KEY_LEFT
#define DGK_RIGHT	KEY_RIGHT
#define DGK_SELECT	KEY_SELECT
#define DGK_CANCEL	KEY_CANCEL
#define DGK_VOLDOWN	KEY_VOLUMEDOWN
#define DGK_VOLUP	KEY_VOLUMEUP
#define DGK_SOFT1	KEY_KBD_LCD_MENU1
#define DGK_SOFT2	KEY_KBD_LCD_MENU2
#define DGK_SOFT3	KEY_KBD_LCD_MENU3
#define DGK_SOFT4	KEY_KBD_LCD_MENU4
#define DGK_MUTE	KEY_MICMUTE
#define DGK_MSGS	KEY_VOICEMAIL
#define DGK_INFO	KEY_INFO
/*
 * Hookswitch: function confirmed, code is a choice.  There is no hook switch
 * in the SW_* list and KEY_PICKUP_PHONE/KEY_HANGUP_PHONE are momentary.
 * As under the vendor kernel, the key is down while the handset is on the
 * cradle and released while it is lifted.
 */
#define DGK_HOOK	KEY_PHONE
/*
 * Function confirmed the same way, but Linux has no matching code:
 * placeholders.  The numbers carry no meaning.
 */
#define DGK_SPEAKER	KEY_MACRO1
#define DGK_HEADSET	KEY_MACRO2
#define DGK_TRANSFER	KEY_MACRO3
#define DGK_HOLD	KEY_MACRO4
#define DGK_REDIAL	KEY_MACRO6
#define DGK_CONF	KEY_MACRO15
#define DGK_LINE1	KEY_MACRO10
#define DGK_LINE2	KEY_MACRO9
#define DGK_LINE3	KEY_MACRO16
#define DGK_LINE4	KEY_MACRO11
#define DGK_BLF1	KEY_MACRO17
#define DGK_BLF2	KEY_MACRO18
#define DGK_BLF3	KEY_MACRO19
#define DGK_BLF4	KEY_MACRO20
#define DGK_BLF5	KEY_MACRO21
#define DGK_BLF6	KEY_MACRO22
#define DGK_BLF7	KEY_MACRO5
#define DGK_BLF8	KEY_MACRO12
#define DGK_BLF9	KEY_MACRO8
#define DGK_BLF10	KEY_MACRO13
/*
 * Function from the vendor phone application's key table (see the top of
 * this file; characters that occur only in the D6x and D70 tables), and
 * Linux has a code for it.
 */
#define DGK_MENU	KEY_MENU
#define DGK_APPS	KEY_APPSELECT
#define DGK_CONTACTS	KEY_ADDRESSBOOK
/* The same, but Linux has no matching code: placeholders. */
#define DGK_STATUS	KEY_MACRO24
#define DGK_BLF_LEFT	KEY_MACRO26
#define DGK_BLF_RIGHT	KEY_MACRO27
#define DGK_LINE5	KEY_MACRO28
#define DGK_LINE6	KEY_MACRO29

#endif /* DIGIUM_KEYS_PROPOSED */

#endif /* DIGIUM_KEYS_H */
