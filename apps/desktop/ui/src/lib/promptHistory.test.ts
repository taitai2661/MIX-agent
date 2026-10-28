import { describe, expect, it, beforeEach } from "vitest";
import {
  clearPromptHistory,
  loadPromptHistory,
  promptHistoryCount,
  pushPromptHistory,
} from "./promptHistory";

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

describe("promptHistory", () => {
  let store: MemoryStorage;

  beforeEach(() => {
    store = new MemoryStorage();
  });

  it("loads empty array when store is empty", () => {
    expect(loadPromptHistory(store)).toEqual([]);
  });

  it("pushes prompt and returns most recent first", () => {
    pushPromptHistory("hello", store);
    pushPromptHistory("world", store);
    expect(loadPromptHistory(store)).toEqual(["world", "hello"]);
  });

  it("ignores whitespace-only strings", () => {
    pushPromptHistory("first", store);
    pushPromptHistory("   ", store);
    expect(loadPromptHistory(store)).toEqual(["first"]);
  });

  it("deduplicates existing entries and brings to front", () => {
    pushPromptHistory("first", store);
    pushPromptHistory("second", store);
    pushPromptHistory("first", store);
    expect(loadPromptHistory(store)).toEqual(["first", "second"]);
  });

  it("clears and counts history", () => {
    pushPromptHistory("first", store);
    pushPromptHistory("second", store);
    expect(promptHistoryCount(store)).toBe(2);
    clearPromptHistory(store);
    expect(loadPromptHistory(store)).toEqual([]);
    expect(promptHistoryCount(store)).toBe(0);
  });
});
