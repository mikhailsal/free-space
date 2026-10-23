import { describe, expect, test } from "bun:test";
import { ancestors, format, isUnder, parentOf, parentPlace, parse, relative, sameView, type Route } from "../../src/router";

const base: Route = { path: "/home/tass", mode: "browse" };

describe("hash round trip", () => {
  test("token-only hash from the CLI", () => {
    const r = parse("#t=abc");
    expect(r.path).toBe("");
    expect(r.mode).toBe("browse");
    expect(r.t).toBe("abc");
  });

  test("gui PATH puts the folder before the token", () => {
    const r = parse("#/home/tass/Downloads?t=abc");
    expect(r.path).toBe("/home/tass/Downloads");
    expect(r.t).toBe("abc");
  });

  test("every field survives format → parse", () => {
    const r: Route = {
      path: "/home/tass/My Files/ünï?#%",
      mode: "find",
      sel: "/home/tass/My Files/ünï?#%/a b.iso",
      kind: "file",
      min: "100M",
      name: "*.iso",
      icase: true,
      before: "2025-01-01",
      after: "180d",
      run: true,
      limit: 60,
    };
    const back = parse(format(r));
    expect(back).toEqual({ ...r, raw: undefined, sort: undefined, untouched: undefined, t: undefined });
  });

  test("slashes stay readable, other characters are escaped", () => {
    const h = format({ ...base, path: "/home/tass/a b", sel: "/home/tass/a b/c" });
    expect(h).toBe("#/home/tass/a%20b?sel=/home/tass/a+b/c");
  });

  test("defaults are left out", () => {
    expect(format(base)).toBe("#/home/tass");
    expect(format({ ...base, path: "" })).toBe("#/");
  });

  test("garbage falls back to defaults", () => {
    const r = parse("#/x?mode=evil&limit=-5&untouched=abc&icase=yes");
    expect(r.mode).toBe("browse");
    expect(r.limit).toBeUndefined();
    expect(r.untouched).toBeUndefined();
    expect(r.icase).toBe(false);
  });

  test("bad percent escapes do not throw", () => {
    expect(parse("#/a%zz").path).toBe("/a%zz");
  });

  test("untouched 0 (explicitly off) is kept", () => {
    expect(parse(format({ ...base, mode: "top", kind: "dir", untouched: 0 })).untouched).toBe(0);
  });
});

test("sameView ignores selection and token only", () => {
  expect(sameView({ ...base, sel: "/a", t: "x" }, { ...base, sel: "/b" })).toBe(true);
  expect(sameView(base, { ...base, mode: "top" })).toBe(false);
  expect(sameView(base, { ...base, sort: "mtime" })).toBe(false);
  expect(sameView(base, { ...base, limit: 60 })).toBe(false);
});

test("path helpers", () => {
  expect(parentOf("/home/tass")).toBe("/home");
  expect(parentOf("/home")).toBe("/");
  expect(parentOf("/")).toBeNull();
  expect(relative("/home/tass/a/b.iso", "/home/tass")).toBe("a/b.iso");
  expect(relative("/etc/x", "/")).toBe("etc/x");
  expect(isUnder("/home/tass/x", "/home/tass")).toBe(true);
  expect(isUnder("/home/tassel", "/home/tass")).toBe(false);
  expect(isUnder("/anything", "/")).toBe(true);
});

describe("places", () => {
  test("plain paths go up by name", () => {
    expect(ancestors({ path: "/home/tass" })).toEqual([{ path: "/" }, { path: "/home" }, { path: "/home/tass" }]);
    expect(parentPlace({ path: "/" })).toBeNull();
    expect(parentPlace({ path: "/home" })).toEqual({ path: "/" });
  });

  test("non-UTF-8 paths keep their bytes on the way up", () => {
    // /data/<ff>/sub: the parent still needs raw, the grandparent does not
    const p = { path: "/data/\ufffd/sub", raw: "2f646174612fff2f737562" };
    expect(parentPlace(p)).toEqual({ path: "/data/\ufffd", raw: "2f646174612fff" });
    expect(ancestors(p).map((a) => a.raw ?? a.path)).toEqual(["/", "/data", "2f646174612fff", "2f646174612fff2f737562"]);
  });

  test("the token is read from the hash but never written back", () => {
    const r = parse("#/home?t=abc");
    expect(r.t).toBe("abc");
    expect(format(r)).toBe("#/home");
  });
});
