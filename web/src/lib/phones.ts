// Drawings of each model's front panel, as data for pages/Screen.tsx.
//
// Reference material (kept outside the repository), by model:
// - D40, D45: retailer top-view photograph of a D40; Digium "Welcome to
//   the Digium D40 IP Phone" quick reference (613-00018B) drawing; the
//   D40/D45 photograph in the Digium phones datasheet.  No photograph of
//   a D45 by itself was found; the datasheet shows one body for both.
// - D50: retailer photograph and the datasheet photograph.
// - D70: the datasheet photograph (taken at an angle).
// - D60, D62: retailer photograph of a D62 with its lamps lit.
// - D65: retailer top-view photographs, with and without the handset;
//   "Digium D6X Series IP Phone Quick Start Guide" (613-00026) drawing.
//
// What those show, and this file follows:
// - Old family (D40, D45, D50, D70): charcoal body; a silver display
//   housing that rises above the body's top edge; teal soft keys and
//   silver line keys on the housing, each joined to the display by a
//   printed line; silver headset, speaker, and mute keys carrying icons;
//   black dial and feature keys.  D40/D45/D50 feature keys carry words
//   (Msgs, Transfer, Redial, Hold, Conf, Info); the D70's carry icons.
//   The check key is the center of the navigation pad.
// - New family (D60, D62, D65): black, in two panels: a glossy upper panel
//   with the display, the silver line and soft keys, and the logo; a matte
//   lower panel with the keys.  Every legend is an icon.  The navigation
//   pad is one disc with no center key; check and X share a bar above it.
//
// Not established from the references: the printed lines from the line
// keys to the display on the new family (none is visible, so none is
// drawn); which of the D70's top-row keys is Apps and which is Menu
// (assigned from the icons: open box, check list); that the key beside the
// D70's pad is Status ("no entry" icon); a D62's page key (drawn without,
// like the D60).
//
// Units are those of the reference photograph each family was measured
// from; the page scales the drawing.

export type Legend =
  | { icon: string }
  | { text: string }
  /// Dial pad: letters above the digit.
  | { digit: string; letters?: string };

export type PanelKey = {
  /// Key name as the ScreenService uses it ("line1", "sk2", "vup").
  id: string;
  cx: number;
  cy: number;
  w: number;
  h: number;
  /// Corner radius; half the height for an oval, large for a round key.
  r: number;
  fill: string;
  /// Legend color.
  ink?: string;
  legend?: Legend;
  /// Legend size, in units.
  size?: number;
  /// Lamp behind the key, by the lamp's key name in the stream.  The whole
  /// key lights.
  lamp?: string;
  /// No outline of its own: part of a larger drawn control (the new
  /// family's navigation disc; halves of a bar).
  bare?: boolean;
};

export type Shape = { d: string; fill: string; stroke?: string; opacity?: number; gloss?: boolean };

export type Panel = {
  viewBox: [number, number, number, number];
  /// Factor from units to CSS pixels at which the display is 1:1 with the
  /// phone's own pixels times `pixelScale`.
  unit: number;
  /// Body parts, back to front, under the keys.
  shapes: Shape[];
  /// Printed guide lines: x1, y1, x2, y2.
  lines: [number, number, number, number][];
  lineColor: string;
  display: { x: number; y: number; w: number; h: number };
  side?: { x: number; y: number; w: number; h: number };
  mwi: { x: number; y: number; w: number; h: number; off: string };
  logo: { x: number; y: number; size: number; fill: string; dot?: string };
  handset: { x: number; y: number; w: number; h: number; fill: string; print?: string };
  /// Speaker grille in the cradle, uncovered when the handset is lifted.
  grille?: { cx: number; cy: number; r: number };
  keys: PanelKey[];
};

const rect = (x: number, y: number, w: number, h: number, r: number) =>
  `M${x + r} ${y}H${x + w - r}Q${x + w} ${y} ${x + w} ${y + r}V${y + h - r}Q${x + w} ${y + h} ${x + w - r} ${y + h}H${x + r}Q${x} ${y + h} ${x} ${y + h - r}V${y + r}Q${x} ${y} ${x + r} ${y}Z`;

const letters: Record<string, string> = { "2": "ABC", "3": "DEF", "4": "GHI", "5": "JKL", "6": "MNO", "7": "PQRS", "8": "TUV", "9": "WXYZ" };
const digits = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "*", "0", "#"];

// ------------------------------------------------------------ new family

const BLACK = "#17181a";
const SILVER = "#d3d6d9";

function newFamily(lineKeys: number, pageKey: boolean): Panel {
  const keys: PanelKey[] = [];
  const white = "#f2f3f4";

  // Line keys: silver ovals right of the display, staggered.
  const linePos = [
    [655, 168],
    [697, 190],
    [655, 215],
    [697, 240],
    [655, 265],
    [697, 289],
  ];
  for (let i = 0; i < lineKeys; i++) {
    keys.push({ id: `line${i + 1}`, cx: linePos[i][0], cy: linePos[i][1], w: 30, h: 16, r: 8, fill: SILVER, lamp: `line${i + 1}` });
  }
  // D65: one silver bar under the line keys pages the rapid-dial keys.
  if (pageKey) keys.push({ id: "blfright", cx: 680, cy: 327, w: 64, h: 15, r: 7.5, fill: SILVER });

  // Soft keys: silver ovals under the display.
  const softX = [355, 427, 500, 573];
  softX.forEach((cx, i) => keys.push({ id: `sk${i + 1}`, cx, cy: 378, w: 42, h: 15, r: 7.5, fill: SILVER }));

  // Headset, speaker, mute: silver ovals with gray icons.
  [
    ["hdst", "headset", 462],
    ["spkr", "speaker", 514],
    ["mute", "mute", 566],
  ].forEach(([id, icon, cy]) =>
    keys.push({ id: id as string, cx: 327, cy: cy as number, w: 46, h: 22, r: 11, fill: SILVER, ink: "#69727d", legend: { icon: icon as string }, size: 18, lamp: id as string }),
  );

  // Dial pad: black keys, white print.
  digits.forEach((d, i) =>
    keys.push({ id: d, cx: 409 + (i % 3) * 59, cy: 462 + Math.floor(i / 3) * 51.5, w: 46, h: 35, r: 6, fill: BLACK, ink: white, legend: { digit: d, letters: letters[d] } }),
  );
  // Volume: one black bar under the dial pad.
  keys.push({ id: "vdn", cx: 438, cy: 670, w: 62, h: 20, r: 3, fill: BLACK, ink: white, legend: { icon: "minus" }, size: 14, bare: true });
  keys.push({ id: "vup", cx: 500, cy: 670, w: 62, h: 20, r: 3, fill: BLACK, ink: white, legend: { icon: "plus" }, size: 14, bare: true });

  // Check and X: the two ends of one bar above the pad.
  keys.push({ id: "select", cx: 623, cy: 446, w: 46, h: 20, r: 3, fill: BLACK, ink: white, legend: { icon: "check" }, size: 13, bare: true });
  keys.push({ id: "cancel", cx: 669, cy: 446, w: 46, h: 20, r: 3, fill: BLACK, ink: white, legend: { icon: "x" }, size: 12, bare: true });
  // Navigation: one disc, four directions, no center key.
  const nav: [string, string, number, number][] = [
    ["up", "up", 648, 490],
    ["left", "left", 624, 514],
    ["right", "right", 672, 514],
    ["down", "down", 648, 538],
  ];
  nav.forEach(([id, icon, cx, cy]) => keys.push({ id, cx, cy, w: 26, h: 26, r: 13, fill: BLACK, ink: white, legend: { icon }, size: 14, bare: true }));
  // Messages, transfer, redial, hold: black ovals, white icons.
  const feat: [string, number, number][] = [
    ["msgs", 610, 590],
    ["xfer", 688, 590],
    ["redial", 610, 637],
    ["hold", 688, 637],
  ];
  feat.forEach(([id, cx, cy]) => keys.push({ id, cx, cy, w: 56, h: 25, r: 12.5, fill: BLACK, ink: white, legend: { icon: id }, size: 17 }));

  return {
    viewBox: [-6, 44, 780, 712],
    unit: 480 / 303,
    shapes: [
      // Cradle wing, left of the two panels.
      { d: "M276 170H96Q62 170 58 204L34 612Q30 664 84 680L276 706Z", fill: "#292b2e" },
      // Recesses the handset's ends rest in.
      { d: rect(84, 184, 152, 74, 26), fill: "#1d1e21" },
      { d: "M96 660Q98 560 166 560Q236 560 240 690Z", fill: "#1d1e21" },
      // Upper panel: glossy black.
      { d: "M290 112H736Q758 112 758 134V412H268V134Q268 112 290 112Z", fill: "#0e0f11", gloss: true },
      // Lower panel: matte.
      { d: "M268 410H758V693Q758 715 736 715H290Q268 715 268 693Z", fill: "#2a2c2f" },
      // The joined bars and the navigation disc.
      { d: rect(407, 660, 124, 20, 10), fill: BLACK, stroke: "#050506" },
      { d: "M469 662V678", fill: "none", stroke: "#3a3c40" },
      { d: rect(600, 436, 92, 20, 10), fill: BLACK, stroke: "#050506" },
      { d: "M646 438V454", fill: "none", stroke: "#3a3c40" },
      { d: "M648 477a37 37 0 1 0 .01 0Z", fill: BLACK, stroke: "#050506" },
    ],
    // A printed tick above each soft key, up to the display.
    lines: softX.map((cx) => [cx, 334, cx, 366]),
    lineColor: "#5d6066",
    display: { x: 312, y: 155, w: 303, h: 303 * (272 / 480) },
    mwi: { x: 678, y: 109, w: 28, h: 7, off: "#b9bdc2" },
    logo: { x: 680, y: 382, size: 19, fill: "#f2f3f4" },
    handset: { x: 50, y: 63, w: 174, h: 669, fill: "#222326" },
    grille: { cx: 152, cy: 400, r: 68 },
    keys,
  };
}

// ------------------------------------------------------------ old family

const CHARCOAL = "#34373c";
const KEY = "#17181b";
const HOUSING_KEY = "#eceef0";
const TEAL = "#5f9db8";
const OLD_SILVER = "#d5d8db";

type Old = {
  lineKeys: number;
  /// Feature key rows under the pad; three columns on the D50 and D70.
  features: string[][];
  /// Words (D40, D45, D50) or icons (D70) on the feature keys.
  words: boolean;
  top?: string[];
  beside?: string;
  wing?: "paper" | "display";
};

const words: Record<string, string> = { msgs: "Msgs", xfer: "Transfer", redial: "Redial", hold: "Hold", conf: "Conf", info: "Info" };

function oldFamily(v: Old): Panel {
  const keys: PanelKey[] = [];
  const white = "#f2f3f4";
  const wide = v.features[0].length === 3;
  // Body and housing grow to the right for the three-column models.
  const grow = wide ? 52 : 0;
  const right = 712 + grow;
  const tall = v.top ? 26 : 0;
  const bottom = 622 + tall;

  // Line keys on the housing, right of the display, staggered; a printed
  // line runs from each to the display bezel.
  const lines: [number, number, number, number][] = [];
  const step = v.lineKeys > 4 ? 15 : v.lineKeys > 2 ? 19 : 17;
  const y0 = v.lineKeys > 2 ? 204 : 214;
  for (let i = 0; i < v.lineKeys; i++) {
    const cx = i % 2 ? 610 : 577;
    const cy = y0 + i * step;
    keys.push({ id: `line${i + 1}`, cx, cy, w: 27, h: 14, r: 7, fill: HOUSING_KEY, lamp: `line${i + 1}` });
    lines.push([550, cy, cx - 13.5, cy]);
  }

  // Soft keys: teal, under the display, each with a line up to it.
  const softX = [365, 413, 462, 511];
  softX.forEach((cx, i) => {
    keys.push({ id: `sk${i + 1}`, cx, cy: 338, w: 36, h: 15, r: 5, fill: TEAL });
    lines.push([cx, 308, cx, 330.5]);
  });

  // Headset, speaker, mute: silver keys with dark icons.
  [
    ["hdst", "headset", 336, 402],
    ["spkr", "speaker", 334, 452],
    ["mute", "mute", 332, 502],
  ].forEach(([id, icon, cx, cy]) =>
    keys.push({ id: id as string, cx: cx as number, cy: cy as number, w: 37, h: 28, r: 4, fill: OLD_SILVER, ink: "#555b63", legend: { icon: icon as string }, size: 20, lamp: id as string }),
  );

  // Dial pad.
  digits.forEach((d, i) =>
    keys.push({ id: d, cx: 394 + (i % 3) * 47, cy: 402 + Math.floor(i / 3) * 40, w: 37, h: 29, r: 4, fill: KEY, ink: white, legend: { digit: d, letters: d === "0" ? "OPER" : letters[d] } }),
  );
  // Volume: one bar.
  keys.push({ id: "vdn", cx: 414, cy: 566, w: 56, h: 16, r: 3, fill: KEY, ink: white, legend: { icon: "minus" }, size: 11, bare: true });
  keys.push({ id: "vup", cx: 470, cy: 566, w: 56, h: 16, r: 3, fill: KEY, ink: white, legend: { icon: "plus" }, size: 11, bare: true });

  // Right of the dial pad.
  const fx = wide ? [566, 626, 686] : [568, 636];
  const navX = wide ? 606 : 590;
  let y = 400;
  if (v.top) {
    v.top.forEach((id, i) => keys.push({ id, cx: fx[i], cy: y, w: 44, h: 22, r: 4, fill: KEY, ink: white, legend: { icon: id }, size: 16 }));
    y += 44;
  }
  // Navigation: five round keys, check in the center; X above and right.
  const nav: [string, string, number, number][] = [
    ["up", "up", navX, y],
    ["cancel", "x", navX + 34, y - 2],
    ["left", "left", navX - 32, y + 30],
    ["select", "check", navX, y + 30],
    ["right", "right", navX + 32, y + 30],
    ["down", "down", navX, y + 60],
  ];
  nav.forEach(([id, icon, cx, cy]) => keys.push({ id, cx, cy, w: 23, h: 23, r: 11.5, fill: KEY, ink: white, legend: { icon }, size: 11 }));
  if (v.beside) keys.push({ id: v.beside, cx: fx[2], cy: y + 66, w: 44, h: 22, r: 4, fill: KEY, ink: white, legend: { icon: v.beside }, size: 16 });
  const fy = y + 103;
  v.features.forEach((row, r) =>
    row.forEach((id, i) =>
      keys.push({
        id,
        cx: fx[i],
        cy: fy + r * 40,
        w: 48,
        h: 24,
        r: 4,
        fill: KEY,
        ink: white,
        legend: v.words ? { text: words[id] } : { icon: id },
        size: v.words ? 9.5 : 16,
      }),
    ),
  );

  const shapes: Shape[] = [
    // Cradle wing under the handset.
    { d: `M290 232H132Q104 232 103 262L98 ${bottom - 24}Q98 ${bottom + 2} 128 ${bottom + 2}H300Z`, fill: "#2b2e32" },
    { d: rect(118, 246, 132, 70, 24), fill: "#1f2124" },
    { d: rect(116, bottom - 112, 136, 96, 30), fill: "#1f2124" },
    // Body.
    { d: `M302 212H${right - 44}Q${right - 22} 212 ${right - 18} 236L${right} ${bottom - 34}Q${right + 2} ${bottom} ${right - 32} ${bottom}H300Q266 ${bottom} 268 ${bottom - 34}L284 236Q286 212 302 212Z`, fill: CHARCOAL },
    // Lighter rim along the bottom edge.
    { d: `M270 ${bottom - 22}Q${(268 + right) / 2} ${bottom + 6} ${right} ${bottom - 22}Q${right + 2} ${bottom} ${right - 32} ${bottom}H300Q268 ${bottom} 270 ${bottom - 22}Z`, fill: "#4a4e55" },
  ];
  // Right wing.
  let side: Panel["side"];
  let vbRight = right + 28;
  if (v.wing) {
    const wx = right + 8;
    shapes.push({ d: `M${right - 30} 262H${wx + 132}Q${wx + 150} 262 ${wx + 150} 282V${bottom - 20}Q${wx + 150} ${bottom} ${wx + 130} ${bottom}H${right - 30}Z`, fill: "#1b1c1f" });
    if (v.wing === "paper") {
      // D50: paper label strip, ten silver keys to its right.
      shapes.push({ d: rect(wx + 18, 290, 78, 290, 2), fill: "#f1f2f4" });
      for (let i = 0; i < 10; i++) {
        if (i % 2 === 0) shapes.push({ d: rect(wx + 18, 290 + i * 29, 78, 29, 0), fill: "#d9dce0" });
        keys.push({ id: `blf${i + 1}`, cx: wx + 120, cy: 304.5 + i * 29, w: 22, h: 12, r: 4, fill: OLD_SILVER, lamp: `blf${i + 1}` });
      }
    } else {
      // D70: second display, ten silver keys to its right, two teal page
      // keys under it.
      const sw = 78;
      const sh = sw * (357 / 113);
      side = { x: wx + 20, y: 286, w: sw, h: sh };
      shapes.push({ d: rect(wx + 14, 280, sw + 12, sh + 12, 4), fill: "#2a2c2e" });
      for (let i = 0; i < 10; i++) {
        keys.push({ id: `blf${i + 1}`, cx: wx + 126, cy: 286 + (i + 0.5) * (sh / 11), w: 22, h: 12, r: 4, fill: OLD_SILVER, lamp: `blf${i + 1}` });
      }
      [
        ["blfleft", wx + 38],
        ["blfright", wx + 80],
      ].forEach(([id, cx]) => {
        keys.push({ id: id as string, cx: cx as number, cy: 286 + sh + 26, w: 30, h: 13, r: 5, fill: TEAL });
        lines.push([cx as number, 286 + sh + 7, cx as number, 286 + sh + 19]);
      });
    }
    vbRight = wx + 164;
  }
  const hr = 662 + grow; // housing right edge
  shapes.push(
    // Silver display housing, rising above the body's top edge.
    { d: `M320 158H${hr - 22}Q${hr - 2} 158 ${hr} 178L${hr + 8} 348Q${(300 + hr) / 2} 380 298 348L306 178Q308 158 320 158Z`, fill: "#c9ccd0", gloss: true, stroke: "#9da1a6" },
    // Display bezel.
    { d: rect(326, 188, 224, 122, 10), fill: "#2b2d2f" },
    // The volume bar and the ring around the pad.
    { d: rect(386, 558, 112, 16, 7), fill: KEY, stroke: "#0b0c0d" },
    { d: "M442 560V572", fill: "none", stroke: "#3a3c40" },
    { d: `M${navX} ${y - 10}a40 40 0 1 0 .01 0Z`, fill: "none", stroke: "#26282c" },
  );

  return {
    viewBox: [70, 76, vbRight - 70, bottom + 22 - 76],
    unit: (v.wing === "display" ? 320 : 240) / 196,
    shapes,
    lines,
    lineColor: "#8b9097",
    display: { x: 340, y: 200, w: 196, h: 98 },
    side,
    mwi: { x: hr - 36, y: 162, w: 26, h: 14, off: "#a3abba" },
    logo: { x: 598 + grow / 2, y: 196, size: 16, fill: "#7a828c", dot: "#e8742c" },
    handset: { x: 106, y: 96, w: 152, h: bottom - 96 - 14, fill: "#303338", print: "HDVoice" },
    keys,
  };
}

const two = [
  ["msgs", "xfer"],
  ["redial", "hold"],
];
const three = [
  ["msgs", "xfer", "conf"],
  ["redial", "hold", "info"],
];

export const panels: Record<string, Panel> = {
  D40: oldFamily({ lineKeys: 2, features: two, words: true }),
  D45: oldFamily({ lineKeys: 2, features: two, words: true }),
  D50: oldFamily({ lineKeys: 4, features: three, words: true, wing: "paper" }),
  D70: oldFamily({ lineKeys: 6, features: three, words: false, top: ["apps", "dir", "menu"], beside: "status", wing: "display" }),
  D60: newFamily(2, false),
  D62: newFamily(2, false),
  D65: newFamily(6, true),
};
