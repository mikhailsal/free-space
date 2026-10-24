// Every text pair of the D15 token sheet must reach WCAG AA (4.5:1), read
// from tokens.css itself so a token edit cannot slip past (T32, Pass 6).
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const css = readFileSync(new URL("../../src/tokens.css", import.meta.url), "utf8");
const tok: Record<string, string> = {};
for (const m of css.matchAll(/--([a-z-]+):\s*(#[0-9a-f]{6})\s*;/gi)) tok[m[1]!] = m[2]!;

type Rgb = [number, number, number];
const rgb = (hex: string): Rgb => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as Rgb;
const lum = (c: Rgb) => {
  const [r, g, b] = c.map((v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
};
const ratio = (a: Rgb, b: Rgb) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x! + 0.05) / (y! + 0.05);
};
/** `fg` at `alpha` over `bg`, as the share tint is drawn. */
const over = (fg: Rgb, alpha: number, bg: Rgb): Rgb => fg.map((v, i) => v * alpha + bg[i]! * (1 - alpha)) as Rgb;

const PAIRS: [string, string][] = [
  ["fg", "bg"],
  ["muted", "bg"],
  ["fg", "surface"],
  ["muted", "surface"],
  ["fg", "hover"],
  ["muted", "hover"],
  ["accent", "bg"],
  ["accent", "surface"],
  ["bg", "accent"],
  ["accent", "accent-bg"],
  ["fg", "accent-bg"],
  ["muted", "accent-bg"],
  ["warn-fg", "warn-bg"],
  ["warn-fg", "bg"],
  ["danger-fg", "danger-bg"],
  ["danger-fg", "bg"],
  ["ok-fg", "bg"],
  ["bg", "fg"],
  ["accent-visited", "surface"],
];

test("tokens.css defines every color the sheet names", () => {
  for (const [a, b] of PAIRS) {
    expect(tok[a], a).toBeDefined();
    expect(tok[b], b).toBeDefined();
  }
});

for (const [a, b] of PAIRS) {
  test(`${a} on ${b} ≥ 4.5:1`, () => {
    expect(ratio(rgb(tok[a]!), rgb(tok[b]!))).toBeGreaterThanOrEqual(4.5);
  });
}

test("text on the share tint stays readable (13 % rows, 22 % selected)", () => {
  const accent = rgb(tok.accent!);
  const row = over(accent, 0.13, rgb(tok.bg!));
  const sel = over(accent, 0.22, rgb(tok["accent-bg"]!));
  for (const t of ["fg", "muted", "danger-fg"]) {
    expect(ratio(rgb(tok[t]!), row)).toBeGreaterThanOrEqual(4.5);
  }
  // muted reaches only 4.3:1 on the selected tint: work.css turns it to fg there
  for (const t of ["fg", "danger-fg"]) {
    expect(ratio(rgb(tok[t]!), sel)).toBeGreaterThanOrEqual(4.5);
  }
  expect(ratio(rgb(tok.muted!), sel)).toBeLessThan(4.5);
});

test("work.css keeps muted text off the selected row", () => {
  const work = readFileSync(new URL("../../src/work.css", import.meta.url), "utf8");
  expect(work).toMatch(/tr\.sel td,\s*\.grid tbody tr\.sel \.k,\s*\.grid tbody tr\.sel \.badge \{\s*color: var\(--fg\)/);
});
