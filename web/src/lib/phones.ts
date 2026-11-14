// Front-panel layouts, one table per model, drawn by pages/Screen.tsx.
//
// Sources for the positions:
// - D40, D45: Digium "Welcome to the Digium D40 IP Phone" quick reference
//   (613-00018B), front-panel drawing; the D40/D45 photograph in the
//   Digium phones datasheet (one body for both models).
// - D50, D70: product photographs in the same datasheet.
// - D60, D62, D65: "Digium D6X Series IP Phone Quick Start Guide"
//   (613-00026), numbered front-panel drawing of a D65, with the note
//   that the D60 and D62 have two line keys.
// - Key presses on a D40, a D50, and a D60 recorded in
//   d60-re/kernel-re/gpio-input-leds/NOTES.md: line keys are right of the
//   display, top to bottom; soft keys are under it, left to right; the
//   D50's BLF keys are one column, top to bottom; a D60 has no keys
//   beyond the D40's set.
//
// Not established: which of the D70's four extra feature keys is Apps and
// which is Menu (assigned from the icons in the photograph: open box and
// check list), and that the key beside its navigation pad is Status (a
// "no entry" icon).  Lamp colors are those of the kernel port's lamp
// tables; a D60 has a green headset lamp, a green speaker lamp, and a red
// mute lamp only.
//
// Units are CSS pixels on a faceplate whose display is 480 wide.

export type KeyShape = "pill" | "round" | "rect";

export type PanelKey = {
  /// Key name as the ScreenService uses it ("line1", "sk2", "vup").
  id: string;
  label: string;
  /// Small print above the label (dial pad letters).
  sub?: string;
  x: number;
  y: number;
  w: number;
  h: number;
  shape: KeyShape;
  /// Half of one bar shared with the neighboring key (volume; the D6x
  /// check/X bar): which end of the bar this key is.
  half?: "left" | "right";
  /// Lamp in the key, by the lamp's key name in the stream.
  lamp?: string;
};

export type Panel = {
  width: number;
  height: number;
  /// Main display: position, and the factor its pixels are drawn at.
  display: { x: number; y: number; scale: number };
  /// D70 rapid-dial display.
  side?: { x: number; y: number };
  /// D50 paper label strip beside its rapid-dial keys.
  paper?: { x: number; y: number; w: number; h: number };
  /// Message waiting indicator (lamp "msgs").
  mwi: { x: number; y: number; w: number; h: number };
  keys: PanelKey[];
};

const DX = 24; // display left
const DY = 40; // display top
const DW = 480; // display width as drawn

type Variant = {
  displayHeight: number;
  scale: number;
  lineKeys: number;
  /// D65: the rapid-dial page key under the line keys.
  pageKey?: boolean;
  nav: "center-check" | "check-bar";
  /// Feature key rows under the navigation pad.
  features: string[][];
  /// D70: a row of feature keys above the navigation pad, and one beside it.
  topFeatures?: string[];
  besideNav?: string;
  wing?: "paper" | "display";
};

const labels: Record<string, string> = {
  msgs: "Msgs",
  xfer: "Transfer",
  redial: "Redial",
  hold: "Hold",
  conf: "Conf",
  info: "Info",
  status: "Status",
  dir: "Contacts",
  apps: "Apps",
  menu: "Menu",
};

const letters: Record<string, string> = { "2": "ABC", "3": "DEF", "4": "GHI", "5": "JKL", "6": "MNO", "7": "PQRS", "8": "TUV", "9": "WXYZ" };

function build(v: Variant): Panel {
  const keys: PanelKey[] = [];
  const add = (id: string, label: string, x: number, y: number, w: number, h: number, shape: KeyShape, extra: Partial<PanelKey> = {}) =>
    keys.push({ id, label, x, y, w, h, shape, ...extra });

  // Line keys: right of the display, staggered, top to bottom.
  const lineX = DX + DW + 16;
  const pitch = v.lineKeys > 4 ? 34 : 40;
  const lineY = DY + (v.lineKeys > 2 ? 10 : 24);
  for (let i = 0; i < v.lineKeys; i++) {
    add(`line${i + 1}`, "", lineX + (i % 2 ? 30 : 0), lineY + i * pitch, 34, 18, "pill", { lamp: `line${i + 1}` });
  }
  if (v.pageKey) add("blfright", "", lineX, lineY + 6 * pitch, 64, 16, "pill");

  // Soft keys: under the display, left to right.
  const softY = DY + v.displayHeight + 14;
  for (let i = 0; i < 4; i++) add(`sk${i + 1}`, "", DX + i * 120 + 25, softY, 70, 18, "pill");

  // Lower section.
  const y1 = softY + 52;
  // Headset, speaker, mute: one column left of the dial pad.
  [
    ["hdst", "Headset"],
    ["spkr", "Speaker"],
    ["mute", "Mute"],
  ].forEach(([id, label], i) => add(id, label, DX, y1 + i * 44, 58, 30, "rect", { lamp: id }));
  // Dial pad.
  ["1", "2", "3", "4", "5", "6", "7", "8", "9", "*", "0", "#"].forEach((d, i) =>
    add(d, d, DX + 76 + (i % 3) * 56, y1 + Math.floor(i / 3) * 44, 48, 34, "rect", { sub: letters[d] }),
  );
  // Volume: one bar under the dial pad, down on the left.
  const volY = y1 + 4 * 44 + 6;
  add("vdn", "−", DX + 76, volY, 80, 18, "pill", { half: "left" });
  add("vup", "+", DX + 76 + 80, volY, 80, 18, "pill", { half: "right" });

  // Right of the dial pad: navigation, then the feature keys.
  const cols = Math.max(...v.features.map((r) => r.length), v.topFeatures?.length ?? 0);
  const fx = DX + 276 + (cols === 2 ? 22 : 0);
  const fpitch = 64;
  let y = y1;
  if (v.topFeatures) {
    v.topFeatures.forEach((id, i) => add(id, labels[id], fx + i * fpitch, y, 56, 26, "rect"));
    y += 40;
  }
  const cx = fx + (cols * fpitch - 8) / 2 - (v.nav === "center-check" ? 12 : 0);
  if (v.nav === "check-bar") {
    // New family (D60, D62, D65): no center key.  The check key is above
    // the pad, as the left end of one bar whose right end is the X key
    // (Quick Start Guide item 10, "Confirmation").
    add("select", "✓", cx - 44, y, 44, 18, "pill", { half: "left" });
    add("cancel", "✕", cx, y, 44, 18, "pill", { half: "right" });
    y += 28;
    add("up", "▲", cx - 15, y, 30, 26, "round");
    add("left", "◀", cx - 49, y + 26, 30, 26, "round");
    add("right", "▶", cx + 19, y + 26, 30, 26, "round");
    add("down", "▼", cx - 15, y + 52, 30, 26, "round");
    y += 88;
  } else {
    // Old family (D40, D45, D50, D70): the check key is the center of the
    // pad; the X key is above and to the right of it.
    add("up", "▲", cx - 15, y, 30, 26, "round");
    add("cancel", "✕", cx + 28, y, 30, 26, "round");
    add("left", "◀", cx - 49, y + 30, 30, 26, "round");
    add("select", "✓", cx - 15, y + 30, 30, 26, "round");
    add("right", "▶", cx + 19, y + 30, 30, 26, "round");
    add("down", "▼", cx - 15, y + 60, 30, 26, "round");
    if (v.besideNav) add(v.besideNav, labels[v.besideNav], fx + 2 * fpitch, y + 62, 56, 26, "rect");
    y += 100;
  }
  v.features.forEach((row, r) => row.forEach((id, i) => add(id, labels[id], fx + i * fpitch, y + r * 36, 56, 26, "rect")));
  const lowerBottom = Math.max(y + v.features.length * 36, volY + 24);

  const panel: Panel = {
    width: lineX + 80,
    height: lowerBottom + 20,
    display: { x: DX, y: DY, scale: v.scale },
    mwi: { x: lineX + 34, y: 10, w: 34, h: 12 },
    keys,
  };

  // Right wing: ten rapid-dial keys in one column, top to bottom.
  if (v.wing) {
    const wx = panel.width + 8;
    if (v.wing === "display") {
      // D70: second display, keys to its right, two page keys under it.
      panel.side = { x: wx, y: 70 };
      for (let i = 0; i < 10; i++) add(`blf${i + 1}`, "", wx + 113 + 10, 70 + 8 + i * 32.4, 34, 16, "pill", { lamp: `blf${i + 1}` });
      add("blfleft", "", wx + 6, 70 + 357 + 12, 44, 16, "pill");
      add("blfright", "", wx + 62, 70 + 357 + 12, 44, 16, "pill");
      panel.width = wx + 113 + 10 + 34 + 20;
      panel.height = Math.max(panel.height, 70 + 357 + 12 + 16 + 20);
    } else {
      // D50: paper label strip, keys to its right.
      panel.paper = { x: wx, y: 96, w: 96, h: 300 };
      for (let i = 0; i < 10; i++) add(`blf${i + 1}`, "", wx + 96 + 10, 96 + 7 + i * 30, 34, 16, "pill", { lamp: `blf${i + 1}` });
      panel.width = wx + 96 + 10 + 34 + 20;
      panel.height = Math.max(panel.height, 96 + 300 + 20);
    }
  }
  return panel;
}

const base = [
  ["msgs", "xfer"],
  ["redial", "hold"],
];
const six = [
  ["msgs", "xfer", "conf"],
  ["redial", "hold", "info"],
];

const smallGray: Variant = { displayHeight: 240, scale: 2, lineKeys: 2, nav: "center-check", features: base };
const color: Variant = { displayHeight: 272, scale: 1, lineKeys: 2, nav: "check-bar", features: base };

export const panels: Record<string, Panel> = {
  D40: build(smallGray),
  D45: build(smallGray),
  D50: build({ ...smallGray, lineKeys: 4, features: six, wing: "paper" }),
  D70: build({
    displayHeight: 240,
    scale: 1.5,
    lineKeys: 6,
    nav: "center-check",
    features: six,
    topFeatures: ["apps", "dir", "menu"],
    besideNav: "status",
    wing: "display",
  }),
  D60: build(color),
  D62: build(color),
  D65: build({ ...color, lineKeys: 6, pageKey: true }),
};
