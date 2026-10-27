// Render ANSI-colored terminal output (stdin) as a static SVG "screenshot".
// Usage: FORCE_COLOR=1 aiblame | node scripts/ansi-to-svg.mjs "title" > docs/demo.svg
import { readFileSync } from "node:fs";

const input = readFileSync(0, "utf8").replace(/\r/g, "");
const title = process.argv[2] ?? "";
const CW = 8.4; // char width at 14px monospace
const LH = 20;
const PAD = 20;
const TOP = 44;
const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

const lines = input.split("\n");
const rows = [];
let maxCols = 0;
for (const line of lines) {
  const spans = [];
  let fg = "#c9d1d9";
  let bold = false;
  let dim = false;
  let col = 0;
  const re = /\x1b\[([0-9;]*)m|([^\x1b]+)/g;
  let m;
  while ((m = re.exec(line))) {
    if (m[1] !== undefined) {
      const codes = m[1].split(";").map(Number);
      for (let i = 0; i < codes.length; i++) {
        const c = codes[i];
        if (c === 0) (fg = "#c9d1d9"), (bold = false), (dim = false);
        else if (c === 1) bold = true;
        else if (c === 2) dim = true;
        else if (c === 22) (bold = false), (dim = false);
        else if (c === 39) fg = "#c9d1d9";
        else if (c === 38 && codes[i + 1] === 2) {
          fg = `rgb(${codes[i + 2]},${codes[i + 3]},${codes[i + 4]})`;
          i += 4;
        }
      }
    } else {
      const text = m[2];
      spans.push({ col, text, fg: dim ? "#7d8590" : fg, bold });
      col += [...text].length;
    }
  }
  maxCols = Math.max(maxCols, col);
  rows.push(spans);
}
while (rows.length && rows.at(-1).length === 0) rows.pop();

const W = Math.ceil(maxCols * CW + PAD * 2);
const H = TOP + rows.length * LH + PAD / 2;
const out = [
  `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">`,
  `<rect width="${W}" height="${H}" rx="10" fill="#0d1117"/>`,
  `<circle cx="20" cy="20" r="6" fill="#ff5f57"/><circle cx="40" cy="20" r="6" fill="#febc2e"/><circle cx="60" cy="20" r="6" fill="#28c840"/>`,
  `<text x="${W / 2}" y="24" text-anchor="middle" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace" font-size="12" fill="#7d8590">${esc(title)}</text>`,
  `<g font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,'DejaVu Sans Mono',monospace" font-size="14">`,
];
rows.forEach((spans, r) => {
  for (const s of spans) {
    // place every word at its exact column so alignment never depends on the font
    const chars = [...s.text];
    let i = 0;
    while (i < chars.length) {
      if (chars[i] === " ") {
        i++;
        continue;
      }
      let j = i;
      while (j < chars.length && chars[j] !== " ") j++;
      const word = chars.slice(i, j).join("");
      out.push(
        `<text x="${(PAD + (s.col + i) * CW).toFixed(1)}" y="${TOP + r * LH + 14}" fill="${s.fg}"${s.bold ? ' font-weight="700"' : ""} textLength="${((j - i) * CW).toFixed(1)}" lengthAdjust="spacingAndGlyphs">${esc(word)}</text>`,
      );
      i = j;
    }
  }
});
out.push("</g></svg>");
process.stdout.write(out.join("\n") + "\n");
