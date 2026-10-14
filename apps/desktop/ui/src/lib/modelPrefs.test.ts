import { afterEach, describe, expect, it, vi } from "vitest";
import {
  MAX_RECENT_MODELS,
  getFavoriteModels,
  getLastModelForSurface,
  getRecentModels,
  isFavoriteModel,
  recordRecentModel,
  setLastModelForSurface,
  subscribeModelPrefs,
  toggleFavoriteModel,
} from "./modelPrefs";

afterEach(() => localStorage.clear());

describe("modelPrefs", () => {
  it("toggles favourites", () => {
    expect(isFavoriteModel("m1")).toBe(false);
    toggleFavoriteModel("m1");
    expect(isFavoriteModel("m1")).toBe(true);
    expect(toggleFavoriteModel("m1")).toEqual([]);
    expect(getFavoriteModels()).toEqual([]);
  });

  it("keeps recents newest-first, without duplicates and capped", () => {
    for (let i = 0; i < MAX_RECENT_MODELS + 2; i += 1) recordRecentModel(`m${i}`);
    recordRecentModel("m2");
    const recents = getRecentModels();
    expect(recents[0]).toBe("m2");
    expect(new Set(recents).size).toBe(recents.length);
    expect(recents).toHaveLength(MAX_RECENT_MODELS);
  });

  it("remembers the last model per surface", () => {
    setLastModelForSurface("chat", "a");
    setLastModelForSurface("desktop-agent", "b");
    expect(getLastModelForSurface("chat")).toBe("a");
    expect(getLastModelForSurface("desktop-agent")).toBe("b");
  });

  it("notifies subscribers on a write", () => {
    const listener = vi.fn();
    const unsubscribe = subscribeModelPrefs(listener);
    toggleFavoriteModel("m1");
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
    toggleFavoriteModel("m2");
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("survives malformed stored data", () => {
    localStorage.setItem("mix-agent-model-favorites", "{not json");
    expect(getFavoriteModels()).toEqual([]);
  });
});
