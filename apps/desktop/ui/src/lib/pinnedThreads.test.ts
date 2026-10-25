import { describe, expect, it, beforeEach } from "vitest";
import { loadPinnedThreadIds, togglePinnedThreadId } from "./pinnedThreads";

class MemoryStorage implements Storage {
  private map = new Map<string, string>();
  get length() {
    return this.map.size;
  }
  clear() {
    this.map.clear();
  }
  getItem(key: string) {
    return this.map.get(key) ?? null;
  }
  key(index: number) {
    return Array.from(this.map.keys())[index] ?? null;
  }
  removeItem(key: string) {
    this.map.delete(key);
  }
  setItem(key: string, value: string) {
    this.map.set(key, value);
  }
}

describe("pinnedThreads", () => {
  let store: MemoryStorage;

  beforeEach(() => {
    store = new MemoryStorage();
  });

  it("loads empty array initially", () => {
    expect(loadPinnedThreadIds(store)).toEqual([]);
  });

  it("toggles thread pin on and off", () => {
    togglePinnedThreadId("thread-1", store);
    expect(loadPinnedThreadIds(store)).toEqual(["thread-1"]);

    togglePinnedThreadId("thread-2", store);
    expect(loadPinnedThreadIds(store)).toEqual(["thread-2", "thread-1"]);

    togglePinnedThreadId("thread-1", store);
    expect(loadPinnedThreadIds(store)).toEqual(["thread-2"]);
  });
});
