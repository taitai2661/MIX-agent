import { describe, expect, it } from "vitest";
import { memoryStore } from "./storage";
import {
  loadPaletteUsage,
  PALETTE_USAGE_KEY,
  rankByFrecency,
  recordPaletteUse,
} from "./paletteRecency";

describe("palette frecency", () => {
  it("records activations and ignores malformed storage", () => {
    const store = memoryStore();
    recordPaletteUse("action:settings", 1_000, store);
    recordPaletteUse("action:settings", 2_000, store);
    recordPaletteUse("action:browser", 3_000, store);

    const usage = loadPaletteUsage(store);
    expect(usage["action:settings"]).toEqual({ count: 2, last: 2_000 });
    expect(usage["action:browser"]).toEqual({ count: 1, last: 3_000 });

    store.setItem(PALETTE_USAGE_KEY, JSON.stringify({ bad: "nope" }));
    expect(loadPaletteUsage(store)).toEqual({});
  });

  it("promotes the most recently used rows and keeps the rest in order", () => {
    const store = memoryStore();
    recordPaletteUse("settings:diagnostics", 100, store);
    recordPaletteUse("action:browser", 300, store);
    recordPaletteUse("action:settings", 200, store);

    const items = [
      { id: "action:new-thread" },
      { id: "action:settings" },
      { id: "action:browser" },
      { id: "settings:diagnostics" },
      { id: "thread:1" },
    ];
    const ranked = rankByFrecency(items, loadPaletteUsage(store));
    expect(ranked.slice(0, 3).map((item) => item.id)).toEqual([
      "action:browser",
      "action:settings",
      "settings:diagnostics",
    ]);
    expect(ranked.slice(3).map((item) => item.id)).toEqual([
      "action:new-thread",
      "thread:1",
    ]);
  });

  it("caps how many learned rows may outrank the default order", () => {
    const store = memoryStore();
    for (let i = 0; i < 10; i += 1) {
      recordPaletteUse(`action:${i}`, i + 1, store);
    }
    const items = [
      { id: "fixed" },
      ...Array.from({ length: 10 }, (_, i) => ({ id: `action:${i}` })),
    ];
    const ranked = rankByFrecency(items, loadPaletteUsage(store), 3);
    expect(ranked.slice(0, 3).map((item) => item.id)).toEqual([
      "action:9",
      "action:8",
      "action:7",
    ]);
    expect(ranked[3]?.id).toBe("fixed");
  });
});
