// Drawings of each model's front panel, as data for pages/Screen.tsx.
//
// Reference material (kept outside the repository), by model:
// - D40, D45: retailer top-view and front photographs of a D40; Digium
//   "Welcome to the Digium D40 IP Phone" quick reference (613-00018B)
//   drawing; the D40/D45 photograph in the Digium phones datasheet.  No
//   photograph of a D45 by itself was found; the datasheet shows one body
//   for both.
// - D50: retailer front photograph and the datasheet photograph.
// - D70: retailer front photograph of a D70 with text keys, and photographs
//   at an angle of D70s with icon keys.
// - D60, D62: retailer front photograph of a D60; retailer photograph of a
//   D62 with its lamps lit.
// - D65: retailer top-view photographs, with and without the handset;
//   "Digium D6X Series IP Phone Quick Start Guide" (613-00026) drawing.
// - Sizes: the datasheet's displays (D40, D45, D50: 3.5 inch, 240x120;
//   D70: 4.5 inch, 320x160) and the phones' widths (D40/D45 8.25 inches,
//   D50 10.5 inches, D70 12.5 inches).
//
// What those show, and this file follows:
// - Old family (D40, D45, D50, D70): charcoal body; a silver display
//   housing with rounded corners that rises above the body's top edge;
//   teal soft keys and silver line keys on the housing, each joined to the
//   display by a printed line; silver headset, speaker, and mute keys
//   carrying icons drawn turned; black dial and feature keys.  D40/D45/D50
//   feature keys carry words (Msgs, Transfer, Redial, Hold, Conf, Info);
//   the D70's carry icons.  The check key is the center of the navigation
//   pad.  The D40, D45, and D50 share one housing; the D70's is larger, for
//   the D70's larger display.
// - New family (D60, D62, D65): black, in two panels: a glossy upper panel
//   with the display, the silver line and soft keys, and the logo; a matte
//   lower panel with the keys.  Every legend is an icon.  The navigation
//   pad is one disc with no center key; check and X share a bar above it.
// - Both families: the dial keys (and the old family's headset, speaker,
//   and mute keys) have straight sides and top and bottom edges that bow
//   outward.  Each digit stands below the space for letters, also on the
//   1 key, which has no letters; the 0, star, and pound sit in the middle
//   of their keys.  A handset in a legend is a C-shaped handle with a
//   separate cap at each end.
//
// Not established from the references: the printed lines from the line
// keys to the display on the new family (none is visible, so none is
// drawn); a D62's page key (drawn without, like the D60).
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
  /// Dial pad: letters above the digit.  A key with `letters` (empty on
  /// the 1 key) has its digit below the space for letters; a key without
  /// has its digit in the middle.
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
  /// The top and bottom edges bow outward by this much at the middle; the
  /// sides are straight, and `r` rounds the corners.
  arch?: number;
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

/// Outline of a key with straight sides and top and bottom edges that bow
/// outward by `a` at the middle (each edge a parabola), within the box
/// (x, y, w, h), with corners rounded by `r`.
export function archedRect(x: number, y: number, w: number, h: number, r: number, a: number): string {
  // The top edge from (x, y + a) through (x + w/2, y) to (x + w, y + a):
  // a quadratic with its control point at (x + w/2, y - a).  The part
  // between t0 and t1 is again a quadratic.
  const t0 = r / w;
  const t1 = 1 - t0;
  const edge = (yEnd: number, yCtl: number) => {
    const at = (t: number) => [x + w * t, (1 - t) * (1 - t) * yEnd + 2 * t * (1 - t) * yCtl + t * t * yEnd];
    const slope = (t: number) => 2 * (1 - t) * (yCtl - yEnd) + 2 * t * (yEnd - yCtl);
    const [ax, ay] = at(t0);
    const [bx, by] = at(t1);
    const c = [ax + ((t1 - t0) * w) / 2, ay + ((t1 - t0) * slope(t0)) / 2];
    return { ax, ay, bx, by, c };
  };
  const top = edge(y + a, y - a);
  const bot = edge(y + h - a, y + h + a);
  return (
    `M${x} ${y + a + r}Q${x} ${y + a} ${top.ax} ${top.ay}Q${top.c[0]} ${top.c[1]} ${top.bx} ${top.by}Q${x + w} ${y + a} ${x + w} ${y + a + r}` +
    `V${y + h - a - r}Q${x + w} ${y + h - a} ${bot.bx} ${bot.by}Q${bot.c[0]} ${bot.c[1]} ${bot.ax} ${bot.ay}Q${x} ${y + h - a} ${x} ${y + h - a - r}Z`
  );
}

/// The letters on each dial key; the 1 key has a row for letters with none
/// in it, and the 0, star, and pound keys have no row.
const letters: Record<string, string> = { "1": "", "2": "ABC", "3": "DEF", "4": "GHI", "5": "JKL", "6": "MNO", "7": "PQRS", "8": "TUV", "9": "WXYZ" };
const digits = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "*", "0", "#"];

/// A dial key's legend: the star is drawn, centered; the others are type.
const dialLegend = (d: string): Legend => (d === "*" ? { icon: "asterisk" } : { digit: d, letters: letters[d] });

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
    keys.push({ id: d, cx: 409 + (i % 3) * 59, cy: 462 + Math.floor(i / 3) * 51.5, w: 46, h: 35, r: 3, arch: 1.9, fill: BLACK, ink: white, legend: dialLegend(d), size: 15.5 }),
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
      // Cradle wing, left of the two panels; vertical outer edge.
      { d: "M276 170H88Q62 170 62 196V674Q62 700 88 700H276Z", fill: "#2b2d30" },
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

/// Where the parts of each body are, from the front photographs.  The
/// housing's left edge is at 300 on every model.
type OldLayout = {
  /// The housing's right edge.
  hr: number;
  /// How much higher the housing's top edge is than the D40's.
  lift: number;
  /// The body's top edge and left edge, and (without a rapid-dial
  /// section) the right edge.
  bodyTop: number;
  xl: number;
  xr: number;
  /// How far the headset, speaker, and mute keys, and the dial pad and
  /// volume bar, are moved right from the D40's places.
  dxSide: number;
  dxPad: number;
  display: { x: number; y: number; w: number; h: number };
  bezel: [number, number, number, number];
  softY: number;
  /// The two columns of line keys, the first key's height, and the step.
  lineX: [number, number];
  lineY?: [number, number];
  /// The columns of feature keys, the navigation pad's middle, and the
  /// feature keys' size.
  fx: number[];
  navX: number;
  feature: [number, number];
};

const D40_DISPLAY = { x: 340, y: 200, w: 196, h: 98 };

const layouts: Record<"D40" | "D50" | "D70", OldLayout> = {
  // The body's sides are close to the housing's.
  D40: { hr: 664, lift: 0, bodyTop: 212, xl: 284, xr: 680, dxSide: 0, dxPad: 0, display: D40_DISPLAY, bezel: [326, 188, 224, 122], softY: 338, lineX: [577, 610], fx: [568, 636], navX: 590, feature: [48, 24] },
  // The D40's housing.  The keys sit further left under the housing than
  // on the D40, to make room for three columns of smaller feature keys,
  // the middle column under the navigation pad.
  D50: { hr: 664, lift: 0, bodyTop: 212, xl: 279, xr: 0, dxSide: -21, dxPad: -21, display: D40_DISPLAY, bezel: [326, 188, 224, 122], softY: 338, lineX: [577, 610], fx: [528, 589, 650], navX: 589, feature: [44, 20] },
  // A 4.5-inch display (the D40's: 3.5 inches) in a larger housing that
  // rises higher; the navigation pad's middle is halfway between the first
  // and second columns of feature keys.
  D70: {
    hr: 799, lift: 81, bodyTop: 135, xl: 279, xr: 0, dxSide: 15, dxPad: 29,
    display: { x: 354, y: 157, w: 288, h: 144 }, bezel: [338, 141, 320, 172], softY: 334,
    lineX: [686, 720], lineY: [168, 20.4], fx: [592, 657, 722], navX: 624.5, feature: [48, 24],
  },
};

function oldFamily(v: Old, L: OldLayout): Panel {
  const keys: PanelKey[] = [];
  const white = "#f2f3f4";
  const tall = v.top ? 26 : 0;
  const bottom = 622 + tall;
  const { hr, lift, display: d } = L;
  const [bx, by, bw, bh] = L.bezel;

  // Line keys on the housing, right of the display, staggered; a printed
  // line runs from each to the display bezel.
  const lines: [number, number, number, number][] = [];
  const [y0, step] = L.lineY ?? [v.lineKeys > 2 ? 204 : 214, v.lineKeys > 4 ? 15 : v.lineKeys > 2 ? 19 : 17];
  for (let i = 0; i < v.lineKeys; i++) {
    const cx = L.lineX[i % 2];
    const cy = y0 + i * step;
    keys.push({ id: `line${i + 1}`, cx, cy, w: 27, h: 14, r: 7, fill: HOUSING_KEY, lamp: `line${i + 1}` });
    lines.push([bx + bw, cy, cx - 13.5, cy]);
  }

  // Soft keys: teal, under the display's quarters, each with a line up to
  // the bezel.
  const softX = [0, 1, 2, 3].map((i) => d.x + (d.w * (i + 0.5)) / 4);
  softX.forEach((cx, i) => {
    keys.push({ id: `sk${i + 1}`, cx, cy: L.softY, w: 36, h: 15, r: 5, fill: TEAL });
    lines.push([cx, by + bh - 2, cx, L.softY - 7.5]);
  });

  // Headset, speaker, mute: silver keys, shaped like the dial keys, with
  // dark icons.
  [
    ["hdst", "headset-turned", 402],
    ["spkr", "speaker-turned", 452],
    ["mute", "mute-turned", 502],
  ].forEach(([id, icon, cy]) =>
    keys.push({ id: id as string, cx: 334 + L.dxSide, cy: cy as number, w: 37, h: 28, r: 3.5, arch: 1.3, fill: OLD_SILVER, ink: "#555b63", legend: { icon: icon as string }, size: 20, lamp: id as string }),
  );

  // Dial pad.
  digits.forEach((d, i) =>
    keys.push({ id: d, cx: 394 + L.dxPad + (i % 3) * 47, cy: 402 + Math.floor(i / 3) * 40, w: 37, h: 29, r: 3, arch: 1.4, fill: KEY, ink: white, legend: dialLegend(d), size: 12.9 }),
  );
  // Volume: one bar.
  keys.push({ id: "vdn", cx: 414 + L.dxPad, cy: 566, w: 56, h: 16, r: 3, fill: KEY, ink: white, legend: { icon: "minus" }, size: 11, bare: true });
  keys.push({ id: "vup", cx: 470 + L.dxPad, cy: 566, w: 56, h: 16, r: 3, fill: KEY, ink: white, legend: { icon: "plus" }, size: 11, bare: true });

  // Right of the dial pad.
  const { fx, navX } = L;
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
  const [fw, fh] = L.feature;
  v.features.forEach((row, r) =>
    row.forEach((id, i) =>
      keys.push({
        id,
        cx: fx[i],
        cy: fy + r * 40,
        w: fw,
        h: fh,
        r: 4,
        fill: KEY,
        ink: white,
        legend: v.words ? { text: words[id] } : { icon: id },
        size: v.words ? 9.5 : 16,
      }),
    ),
  );

  const hl = 300; // housing left edge
  // The rapid-dial section (D50, D70) starts at the housing's right edge.
  const wingW = v.wing ? 189 : 0;
  // Outline of body and rapid-dial section together.  The sides are
  // vertical; the bottom edge bows downward.
  const xl = L.xl;
  const xr = v.wing ? hr + wingW : L.xr;
  const bt = L.bodyTop;
  const bow = 16;
  const yb = bottom - 24;
  // Bottom edge: one quadratic from (xl, yb) to (xr, yb), bowed by `bow`.
  // The curve runs between the rounded bottom corners.
  const cl = xl + 20;
  // On the D50 and D70 the curve belongs to the main body alone and ends
  // where the rapid-dial section begins.
  const cr = v.wing ? hr : xr - 20;
  // The bottom edge from `xa` back to the left corner: a quadratic bowed
  // by `bow` between the corners.
  const bottomTo = (xa: number) => {
    const t = (xa - cl) / (cr - cl);
    const cy = yb + 2 * bow * t;
    return `Q${(cl + xa) / 2} ${cy} ${cl} ${yb}Q${xl} ${yb} ${xl} ${yb - 20}`;
  };
  const dp = L.dxPad;
  const shapes: Shape[] = [
    // Cradle wing under the handset.
    { d: rect(98, 232, 200, yb - 232, 24), fill: "#2b2e32" },
    // Upper cup the earpiece rests in, with the hook switch.
    { d: rect(116, 244, 136, 72, 24), fill: "#1f2124" },
    { d: rect(174, 296, 20, 9, 2), fill: "#0e0f10" },
    // Lower cup the mouthpiece rests in.
    { d: rect(116, yb - 100, 136, 88, 28), fill: "#1f2124" },
    // Body.
    {
      d: v.wing
        ? // Body and rapid-dial section as one outline: the section's
          // bottom edge is straight, rising to the right from the end of
          // the body's curve to a rounded bottom-right corner.
          `M${xl + 24} ${bt}H${xr - 24}Q${xr} ${bt} ${xr} ${bt + 24}V${yb - 38}Q${xr} ${yb - 16} ${xr - 22} ${yb - 14}L${hr} ${yb}${bottomTo(hr)}V${bt + 24}Q${xl} ${bt} ${xl + 24} ${bt}Z`
        : `M${xl + 24} ${bt}H${xr - 24}Q${xr} ${bt} ${xr} ${bt + 24}V${yb - 20}Q${xr} ${yb} ${cr} ${yb}${bottomTo(cr)}V${bt + 24}Q${xl} ${bt} ${xl + 24} ${bt}Z`,
      fill: CHARCOAL,
    },
  ];
  // Rapid-dial section: part of the body's own surface (drawn in the body
  // outline above), directly right of the silver housing, holding ten
  // keys beside a label window (D50) or a second display (D70).
  let side: Panel["side"];
  let vbRight = xr + 28;
  if (v.wing) {
    const wx = hr;
    if (v.wing === "paper") {
      // D50: ten label rows behind a narrow window, a small key beside
      // each row.
      const rowH = 24.3;
      const wy = 268;
      shapes.push({ d: rect(wx + 20, wy - 3, 90, 10 * rowH + 6, 2), fill: "#08090a" });
      for (let i = 0; i < 10; i++) {
        shapes.push({ d: rect(wx + 23, wy + i * rowH, 84, rowH, 0), fill: i % 2 ? "#c7cbd1" : "#f3f4f6" });
        keys.push({ id: `blf${i + 1}`, cx: wx + 139, cy: wy + (i + 0.5) * rowH, w: 27, h: 14, r: 5.5, fill: HOUSING_KEY, lamp: `blf${i + 1}` });
      }
    } else {
      // D70: second display (ten entries and a page row), a key beside
      // each entry, two teal page keys under the display.
      const sw = 92;
      const sh = sw * (357 / 113);
      const sy = 256;
      side = { x: wx + 24, y: sy, w: sw, h: sh };
      shapes.push({ d: rect(wx + 20, sy - 4, sw + 8, sh + 8, 2), fill: "#08090a" });
      for (let i = 0; i < 10; i++) {
        keys.push({ id: `blf${i + 1}`, cx: wx + 160, cy: sy + (i + 0.5) * (sh / 11), w: 38, h: 15, r: 6, fill: HOUSING_KEY, lamp: `blf${i + 1}` });
      }
      [
        ["blfleft", wx + 24 + sw * 0.27],
        ["blfright", wx + 24 + sw * 0.73],
      ].forEach(([id, cx]) => {
        keys.push({ id: id as string, cx: cx as number, cy: sy + sh + 24, w: 32, h: 13, r: 5, fill: TEAL });
        lines.push([cx as number, sy + sh + 5, cx as number, sy + sh + 17]);
      });
    }
    vbRight = xr + 24;
  }
  const mid = (hl + hr) / 2;
  // The housing's bottom edge bows downward between rounded corners: a
  // quadratic from (hr - rb, y1) to (hl + rb, y1), and at each corner a
  // quadratic whose control point is where the side meets the bottom
  // edge's tangent, so that the corner joins both smoothly.
  const rb = 20;
  const y1 = 348;
  const yc = 378;
  const yk = y1 - (rb * (yc - y1)) / (hr - rb - mid);
  const T = (y: number) => y - lift;
  shapes.push(
    // Silver display housing, rising above the body's top edge: arched
    // top edge, rounded corners, bowed bottom edge.
    {
      d: `M${hl} ${T(182)}Q${hl} ${T(162)} ${hl + 22} ${T(160)}Q${mid} ${T(148)} ${hr - 22} ${T(160)}Q${hr} ${T(162)} ${hr} ${T(182)}V${yk - rb}Q${hr} ${yk} ${hr - rb} ${y1}Q${mid} ${yc} ${hl + rb} ${y1}Q${hl} ${yk} ${hl} ${yk - rb}Z`,
      fill: "#c9ccd0",
      gloss: true,
      stroke: "#9da1a6",
    },
    // Display bezel.
    { d: rect(bx, by, bw, bh, 10), fill: "#2b2d2f" },
    // The volume bar and the ring around the pad.
    { d: rect(386 + dp, 558, 112, 16, 7), fill: KEY, stroke: "#0b0c0d" },
    { d: `M${442 + dp} 560V572`, fill: "none", stroke: "#3a3c40" },
    { d: `M${navX} ${y - 10}a40 40 0 1 0 .01 0Z`, fill: "none", stroke: "#26282c" },
  );

  const vbTop = Math.min(76, 134 - lift);
  return {
    viewBox: [-98, vbTop, vbRight + 98, bottom + 70 - vbTop],
    unit: (v.wing === "display" ? 320 : 240) / d.w,
    shapes,
    lines,
    lineColor: "#8b9097",
    display: d,
    side,
    // The lamp is the housing's top-right corner itself: a smoked lens
    // that follows the corner along the top and down the right edge.
    mwi: { d: `M${hr - 38} ${T(159)}L${hr - 22} ${T(160)}Q${hr} ${T(162)} ${hr} ${T(182)}V${T(188)}H${hr - 38}Z`, off: "#8e94a2" },
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
  D40: oldFamily({ lineKeys: 2, features: two, words: true }, layouts.D40),
  D45: oldFamily({ lineKeys: 2, features: two, words: true }, layouts.D40),
  D50: oldFamily({ lineKeys: 4, features: three, words: true, wing: "paper" }, layouts.D50),
  D70: oldFamily({ lineKeys: 6, features: three, words: false, top: ["apps", "dir", "menu"], beside: "status", wing: "display" }, layouts.D70),
  D60: newFamily(2, false),
  D62: newFamily(2, false),
  D65: newFamily(6, true),
};
