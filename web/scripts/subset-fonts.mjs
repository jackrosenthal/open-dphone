// Makes the JetBrains Mono files the web UI embeds (src/assets/fonts/):
// Regular and Bold, cut down to the characters a phone's web UI and its
// terminal show.  Not part of the build: the output is committed.  Run it
// when changing the character set or the font version:
//
//   pnpm fonts /path/to/JetBrainsMono-2.304/fonts/webfonts
//
// The input is the official release, https://github.com/JetBrains/JetBrainsMono
// (v2.304: JetBrainsMono-2.304.zip, SHA-256
// 6f6376c6ed2960ea8a963cd7387ec9d76e3f629125bc33d1fdcd7eb7012f7bbf), which is
// licensed under the SIL Open Font License 1.1 (src/assets/fonts/OFL.txt).

import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import subsetFont from "subset-font";

// Inclusive code point ranges.
const ranges = [
  [0x0020, 0x007e], // Basic Latin
  [0x00a0, 0x00ff], // Latin-1 Supplement
  [0x2010, 0x2027], // dashes, quotation marks, bullet, ellipsis
  [0x2030, 0x203a], // per mille, primes, single guillemets
  [0x20ac, 0x20ac], // euro sign
  [0x2190, 0x2199], // arrows
  [0x2500, 0x257f], // box drawing (terminal programs)
  [0x2580, 0x259f], // block elements (terminal programs)
  [0x25a0, 0x25a1], // squares
  [0x25b2, 0x25b3], // triangles
  [0x25b6, 0x25b7],
  [0x25bc, 0x25bd],
  [0x25c0, 0x25c1],
  [0x25cb, 0x25cb], // circles
  [0x25cf, 0x25cf],
  [0x2713, 0x2717], // check marks and crosses
  [0xe0a0, 0xe0b3], // Powerline symbols (shell prompts)
  [0xfffd, 0xfffd], // replacement character
];

const text = ranges.map(([a, b]) => Array.from({ length: b - a + 1 }, (_, i) => String.fromCodePoint(a + i)).join("")).join("");
const source = process.argv[2];
if (!source) {
  console.error("usage: pnpm fonts <directory holding JetBrainsMono-Regular.woff2 and JetBrainsMono-Bold.woff2>");
  process.exit(2);
}
const out = path.join(path.dirname(fileURLToPath(import.meta.url)), "../src/assets/fonts");
for (const style of ["Regular", "Bold"]) {
  const name = `JetBrainsMono-${style}.woff2`;
  const font = await readFile(path.join(source, name));
  // No layout closure: glyphs reachable only through substitution (the
  // programming ligatures, which a terminal never forms) are left out.
  const subset = await subsetFont(font, text, { targetFormat: "woff2", noLayoutClosure: true });
  await writeFile(path.join(out, name), subset);
  console.log(`${name}: ${font.length} -> ${subset.length} bytes`);
}
