import { expect, test } from "bun:test";
import { command, refreshCommand, untouched } from "../../src/cli";
import type { Route } from "../../src/router";

const r = (x: Partial<Route>): Route => ({ path: "/home/tass", mode: "browse", ...x });

test("the CLI line mirrors mode and filters", () => {
  expect(command(r({}), "/home/tass")).toBe("freespace ls /home/tass --json");
  expect(command(r({ sort: "mtime" }), "/home/tass")).toBe("freespace ls /home/tass --sort mtime --json");
  expect(command(r({ mode: "top" }), "/home/tass")).toBe("freespace top /home/tass --min-size 100M --json");
  expect(command(r({ mode: "top", untouched: 3, min: "1G" }), "/home/tass")).toBe(
    "freespace top /home/tass --min-size 1G --older-than 90d --json",
  );
  expect(command(r({ mode: "changes" }), "/home/tass")).toBe("freespace diff /home/tass --kind any --json");
  expect(command(r({ mode: "find", name: "*.iso", icase: true, before: "2025-01-01" }), "/my files")).toBe(
    "freespace find '/my files' --iname '*.iso' --kind any --older-than 2025-01-01 --json",
  );
});

test("Top folders default to Untouched 6 months, and the CLI line says so (D18)", () => {
  expect(untouched(r({ mode: "top", kind: "dir" }))).toBe(6);
  expect(untouched(r({ mode: "top" }))).toBe(0);
  expect(untouched(r({ mode: "top", kind: "dir", untouched: 0 }))).toBe(0);
  expect(command(r({ mode: "top", kind: "dir" }), "/x")).toBe(
    "freespace top /x --kind dir --min-size 100M --older-than 180d --json",
  );
});

test("refresh command", () => {
  expect(refreshCommand("/home/tass/a b")).toBe("sudo freespace refresh '/home/tass/a b'");
});
