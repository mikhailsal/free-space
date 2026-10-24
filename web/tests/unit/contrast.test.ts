// Every text pair of the D15 token sheet must reach WCAG AA (4.5:1), read
// from tokens.css itself so a token edit cannot slip past (T32, Pass 6).
import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const css = readFileSync(new URL("../../src/tokens.css", import.meta.url), "utf8");
const tok: Record<string, string> = {};
for (const [, name, hex] of css.matchAll(/--([a-z-]+):\s*(#[0-9a-f]{6})\s*;/gi)) if (name && hex) tok[name] = hex;

type Rgb = [number, number, number];
const rgb = (hex: string): Rgb => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as Rgb;
/** A token's color; fails the test when tokens.css lacks it. */
const color = (name: string): Rgb => {
  const hex = tok[name];
  if (!hex) throw new Error(`tokens.css has no --${name}`);
  return rgb(hex);
};
const lin = (v: number) => {
  const s = v / 255;
  return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
};
const lum = ([r, g, b]: Rgb) => 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
const ratio = (a: Rgb, b: Rgb) => (Math.max(lum(a), lum(b)) + 0.05) / (Math.min(lum(a), lum(b)) + 0.05);
/** `fg` at `alpha` over `bg`, as the share tint is drawn. */
const over = ([r, g, b]: Rgb, alpha: number, [r0, g0, b0]: Rgb): Rgb => [
  r * alpha + r0 * (1 - alpha),
  g * alpha + g0 * (1 - alpha),
  b * alpha + b0 * (1 - alpha),
];

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
    expect(ratio(color(a), color(b))).toBeGreaterThanOrEqual(4.5);
  });
}

test("text on the share tint stays readable (13 % rows, 22 % selected)", () => {
  const accent = color("accent");
  const row = over(accent, 0.13, color("bg"));
  const sel = over(accent, 0.22, color("accent-bg"));
  for (const t of ["fg", "muted", "danger-fg"]) {
    expect(ratio(color(t), row)).toBeGreaterThanOrEqual(4.5);
  }
  // muted reaches only 4.3:1 on the selected tint: work.css turns it to fg there
  for (const t of ["fg", "danger-fg"]) {
    expect(ratio(color(t), sel)).toBeGreaterThanOrEqual(4.5);
  }
  expect(ratio(color("muted"), sel)).toBeLessThan(4.5);
});

test("work.css keeps muted text off the selected row", () => {
  const work = readFileSync(new URL("../../src/work.css", import.meta.url), "utf8");
  expect(work).toMatch(/tr\.sel td,\s*\.grid tbody tr\.sel \.k,\s*\.grid tbody tr\.sel \.badge \{\s*color: var\(--fg\)/);
});
