import { expect, test } from "bun:test";
import * as fmt from "../../src/format";

test("sizes match the CLI (core/src/format.rs)", () => {
  expect(fmt.size(0)).toBe("0 B");
  expect(fmt.size(1023)).toBe("1023 B");
  expect(fmt.size(1024)).toBe("1.0 K");
  expect(fmt.size(10 * 1024)).toBe("10 K");
  expect(fmt.size(6.9 * 2 ** 30)).toBe("6.9 G");
  expect(fmt.size(206.8 * 2 ** 30)).toBe("207 G");
  expect(fmt.size(2 ** 40)).toBe("1.0 T");
});

test("deltas use a true minus and an em dash for zero", () => {
  expect(fmt.delta(0)).toBe("—");
  expect(fmt.delta(2 ** 30)).toBe("+1.0 G");
  expect(fmt.delta(-(2 ** 20))).toBe("−1.0 M");
  expect(fmt.signed(-1500)).toBe("−1\u202f500");
});

test("counts group with narrow no-break spaces", () => {
  expect(fmt.count(2104331)).toBe("2\u202f104\u202f331");
  expect(fmt.count(999)).toBe("999");
});

test("share of a folder", () => {
  expect(fmt.pct(59, 100)).toBe("59%");
  expect(fmt.pct(1, 1000)).toBe("<1%");
  expect(fmt.pct(0, 100)).toBe("0%");
  expect(fmt.pct(5, 0)).toBe("0%");
});

test("ages", () => {
  const now = 1_800_000_000;
  expect(fmt.ago(0, now)).toBe("unknown");
  expect(fmt.ago(now - 3600, now)).toBe("today");
  expect(fmt.ago(now - 3 * 86400, now)).toBe("3 d");
  expect(fmt.ago(now - 120 * 86400, now)).toBe("4 mo");
  expect(fmt.ago(now - 800 * 86400, now)).toBe("2 y");
  expect(fmt.age(40)).toBe("40 s");
  expect(fmt.age(7 * 3600)).toBe("7h");
  expect(fmt.age(9 * 86400)).toBe("9 d");
  expect(fmt.elapsed(72)).toBe("1 m 12 s");
  expect(fmt.elapsed(3900)).toBe("1 h 05 m");
});

test("shell quoting", () => {
  expect(fmt.shellWord("/home/tass")).toBe("/home/tass");
  expect(fmt.shellWord("*.iso")).toBe("'*.iso'");
  expect(fmt.shellWord("it's here")).toBe("'it'\\''s here'");
});
