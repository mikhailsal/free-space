import { beforeEach, expect, test } from "bun:test";
import { Hidden } from "../../src/hidden";

class Memory {
  m = new Map<string, string>();
  get length() {
    return this.m.size;
  }
  key(i: number) {
    return [...this.m.keys()][i] ?? null;
  }
  getItem(k: string) {
    return this.m.get(k) ?? null;
  }
  setItem(k: string, v: string) {
    this.m.set(k, v);
  }
  removeItem(k: string) {
    this.m.delete(k);
  }
}

let store: Memory;
beforeEach(() => {
  store = new Memory();
  (globalThis as { localStorage?: unknown }).localStorage = store;
});

test("hidden paths cover their subtree and survive a reload", () => {
  const h = new Hidden();
  h.load("vm", "s1");
  h.add({ path: "/home/tass/.cache", allocated: 100 });
  h.add({ path: "/home/tass/.cache", allocated: 100 });
  expect(h.all.length).toBe(1);
  expect(h.has("/home/tass/.cache/pip")).toBe(true);
  expect(h.has("/home/tass/.cachet")).toBe(false);
  const again = new Hidden();
  again.load("vm", "s1");
  expect(again.has("/home/tass/.cache")).toBe(true);
});

test("a new snapshot clears the list and purges old keys (D11)", () => {
  store.setItem("other.app.key", "keep");
  const h = new Hidden();
  h.load("vm", "s1");
  h.add({ path: "/a", allocated: 1 });
  h.load("vm", "s2");
  expect(h.all.length).toBe(0);
  expect(store.getItem("freespace.hidden.vm.s1")).toBeNull();
  expect(store.getItem("other.app.key")).toBe("keep");
});

test("folder chip counts direct children only", () => {
  const h = new Hidden();
  h.load("vm", "s1");
  h.add({ path: "/home/tass/a", allocated: 10 });
  h.add({ path: "/home/tass/b", allocated: 5 });
  h.add({ path: "/home/tass/a/deep", allocated: 3 });
  expect(h.under("/home/tass")).toEqual({ count: 2, allocated: 15 });
  h.remove("/home/tass/a");
  expect(h.under("/home/tass")).toEqual({ count: 1, allocated: 5 });
});

test("corrupt storage is ignored", () => {
  store.setItem("freespace.hidden.vm.s1", "{not json");
  const h = new Hidden();
  h.load("vm", "s1");
  expect(h.all.length).toBe(0);
  store.setItem("freespace.hidden.vm.s1", JSON.stringify([{ path: 1 }, { path: "/ok", allocated: 2 }]));
  h.load("vm", "s1");
  expect(h.all).toEqual([{ path: "/ok", allocated: 2 }]);
});
