import { expect, test } from "bun:test";
import { continuity, nearestExisting } from "../../src/snapshot";

test("ancestor walk lands on the nearest folder that still exists (D10)", async () => {
  const live = new Set(["/", "/home", "/home/tass"]);
  const asked: string[] = [];
  const exists = async (p: string) => {
    asked.push(p);
    return live.has(p);
  };
  expect(await nearestExisting("/home/tass/old/deeper", exists)).toBe("/home/tass");
  expect(asked).toEqual(["/home/tass/old/deeper", "/home/tass/old", "/home/tass"]);
  expect(await nearestExisting("/home", exists)).toBe("/home");
  expect(await nearestExisting("/gone", async () => false)).toBeNull();
});

test("walk stops on a real failure", async () => {
  const boom = async () => {
    throw new Error("session");
  };
  await expect(nearestExisting("/a/b", boom)).rejects.toThrow("session");
});

test("continuity toasts", () => {
  expect(continuity("/a/b", "/a", "/a/b/x", [])).toEqual({ kind: "path-gone", from: "/a/b", to: "/a" });
  expect(continuity("/a", "/a", "/a/x", ["/a/y"])).toEqual({ kind: "row-gone", name: "x" });
  expect(continuity("/a", "/a", "/a/x", ["/a/x"])).toEqual({ kind: "same" });
  expect(continuity("/a", "/a", undefined, [])).toEqual({ kind: "same" });
});
