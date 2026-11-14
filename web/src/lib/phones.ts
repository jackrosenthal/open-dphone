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
// Every phone is drawn flat, as seen straight on: where the only
// photograph is at an angle (D50, D70), positions follow the front-on
// drawings and the family's common grid.  No maker's name or mark is
// drawn.
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
  /// Message waiting lamp: outline, and its color when dark.
  mwi: { d: string; off: string };
  handset: { x: number; y: number; w: number; h: number; fill: string; print?: string };
  /// Speaker grille in the cradle, under the handset.
  grille: { cx: number; cy: number; rx: number; ry: number };
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
  // D65: two silver keys side by side under the line keys page the
  // rapid-dial keys.
  if (pageKey) {
    keys.push({ id: "blfleft", cx: 664, cy: 327, w: 31, h: 15, r: 5, fill: SILVER });
    keys.push({ id: "blfright", cx: 696, cy: 327, w: 31, h: 15, r: 5, fill: SILVER });
  }

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
    viewBox: [-176, 44, 950, 760],
    unit: 480 / 303,
    shapes: [
      // Cradle wing, left of the two panels; its outer edge bows outward.
      { d: "M276 170H104Q70 170 62 204Q30 430 62 652Q70 700 120 700H276Z", fill: "#2b2d30" },
      // Upper cup the earpiece rests in, with the hook switch.
      { d: "M92 172H234V226Q234 264 196 264H130Q92 264 92 226Z", fill: "#1c1d20" },
      { d: rect(152, 246, 22, 9, 2), fill: "#0c0d0e" },
      // Lower arch the mouthpiece rests in.
      { d: "M98 700V642Q98 568 164 568Q230 568 230 642V700Z", fill: "#1c1d20" },
      // Upper panel: glossy black.
      { d: "M290 112H736Q758 112 758 134V412H268V134Q268 112 290 112Z", fill: "#0e0f11", gloss: true },
      // Lower panel: matte.
      { d: "M268 410H758V693Q758 715 736 715H290Q268 715 268 693Z", fill: "#2a2c2f" },
      // The left page key (drawn on every D65; it works where the phone
      // reports that key).
      ...(pageKey ? [{ d: rect(648.5, 319.5, 31, 15, 5), fill: SILVER, stroke: "#00000066" }] : []),
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
    // A strip in the top edge of the upper panel, near the right corner.
    mwi: { d: rect(678, 112, 28, 6, 1), off: "#b9bdc2" },
    handset: { x: 50, y: 63, w: 174, h: 669, fill: "#222326" },
    grille: { cx: 164, cy: 410, rx: 62, ry: 62 },
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
    ["hdst", "headset", 334, 402],
    ["spkr", "speaker", 334, 452],
    ["mute", "mute", 334, 502],
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
    { d: rect(98, 232, 200, bottom - 232, 24), fill: "#2b2e32" },
    // Upper cup the earpiece rests in, with the hook switch.
    { d: rect(116, 244, 136, 72, 24), fill: "#1f2124" },
    { d: rect(174, 296, 20, 9, 2), fill: "#0e0f10" },
    // Lower cup the mouthpiece rests in.
    { d: rect(116, bottom - 104, 136, 90, 28), fill: "#1f2124" },
    // Body.
    { d: rect(268, 212, right - 268, bottom - 212, 24), fill: CHARCOAL },
  ];
  // Right wing: ten rapid-dial keys beside a label window (D50) or a
  // second display (D70), in a black housing joined to the body's side.
  let side: Panel["side"];
  let vbRight = right + 28;
  if (v.wing) {
    const wx = right - 4;
    const ww = 150;
    const top = 262;
    // Housing, with a raised inner panel.
    shapes.push({ d: `M${wx} ${top}H${wx + ww - 18}Q${wx + ww} ${top} ${wx + ww} ${top + 18}V${bottom - 20}Q${wx + ww} ${bottom} ${wx + ww - 20} ${bottom}H${wx}Z`, fill: "#17181a", gloss: true });
    shapes.push({ d: rect(wx + 10, top + 24, ww - 22, bottom - top - 40, 5), fill: "#222427", stroke: "#0c0d0e" });
    const kx = wx + ww - 32;
    if (v.wing === "paper") {
      // D50: ten label rows behind a window, a key beside each row.
      const rowH = 27;
      const wy = top + 38;
      shapes.push({ d: rect(wx + 20, wy - 4, 84, 10 * rowH + 8, 3), fill: "#0d0e0f" });
      for (let i = 0; i < 10; i++) {
        shapes.push({ d: rect(wx + 24, wy + i * rowH, 76, rowH, 0), fill: i % 2 ? "#c7cbd1" : "#f3f4f6" });
        keys.push({ id: `blf${i + 1}`, cx: kx, cy: wy + (i + 0.5) * rowH, w: 26, h: 13, r: 6.5, fill: HOUSING_KEY, lamp: `blf${i + 1}` });
      }
    } else {
      // D70: second display (ten entries and a page row), a key beside
      // each entry, two teal page keys under the display.
      const sw = 80;
      const sh = sw * (357 / 113);
      const sy = top + 38;
      side = { x: wx + 22, y: sy, w: sw, h: sh };
      shapes.push({ d: rect(wx + 17, sy - 5, sw + 10, sh + 10, 3), fill: "#0d0e0f" });
      for (let i = 0; i < 10; i++) {
        keys.push({ id: `blf${i + 1}`, cx: kx, cy: sy + (i + 0.5) * (sh / 11), w: 26, h: 13, r: 6.5, fill: HOUSING_KEY, lamp: `blf${i + 1}` });
      }
      [
        ["blfleft", wx + 22 + sw * 0.27],
        ["blfright", wx + 22 + sw * 0.73],
      ].forEach(([id, cx]) => {
        keys.push({ id: id as string, cx: cx as number, cy: sy + sh + 26, w: 30, h: 13, r: 5, fill: TEAL });
        lines.push([cx as number, sy + sh + 6, cx as number, sy + sh + 19]);
      });
    }
    vbRight = wx + ww + 24;
  }
  const hl = 300; // housing left edge
  const hr = 664 + grow; // housing right edge
  shapes.push(
    // Silver display housing, rising above the body's top edge; straight
    // sides, bowed lower edge.
    { d: `M${hl + 22} 158H${hr - 22}Q${hr} 158 ${hr} 180V346Q${(hl + hr) / 2} 378 ${hl} 346V180Q${hl} 158 ${hl + 22} 158Z`, fill: "#c9ccd0", gloss: true, stroke: "#9da1a6" },
    // Display bezel.
    { d: rect(326, 188, 224, 122, 10), fill: "#2b2d2f" },
    // The volume bar and the ring around the pad.
    { d: rect(386, 558, 112, 16, 7), fill: KEY, stroke: "#0b0c0d" },
    { d: "M442 560V572", fill: "none", stroke: "#3a3c40" },
    { d: `M${navX} ${y - 10}a40 40 0 1 0 .01 0Z`, fill: "none", stroke: "#26282c" },
  );

  return {
    viewBox: [-98, 76, vbRight + 98, bottom + 70 - 76],
    unit: (v.wing === "display" ? 320 : 240) / 196,
    shapes,
    lines,
    lineColor: "#8b9097",
    display: { x: 340, y: 200, w: 196, h: 98 },
    side,
    // The lamp is the housing's top-right corner itself: a smoked lens
    // that follows the corner along the top and down the right edge.
    mwi: { d: `M${hr - 36} 158H${hr - 22}Q${hr} 158 ${hr} 180V186H${hr - 36}Z`, off: "#8e94a2" },
    handset: { x: 106, y: 96, w: 152, h: bottom - 96 - 14, fill: "#303338", print: "HDVoice" },
    // Vertical slots beside the housing, as in the quick reference drawing.
    grille: { cx: 184, cy: 400, rx: 40, ry: 58 },
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
