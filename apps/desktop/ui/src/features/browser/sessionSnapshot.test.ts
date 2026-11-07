import { beforeEach, describe, expect, it } from "vitest";
import {
  browserSessionKey,
  clearBrowserSession,
  readBrowserSession,
  saveBrowserSession,
} from "./sessionSnapshot";

const WORKSPACE = "11111111-1111-4111-8111-111111111111";

beforeEach(() => {
  localStorage.clear();
});

describe("browser session snapshot", () => {
  it("round-trips tabs and the active index", () => {
    saveBrowserSession(WORKSPACE, {
      tabs: [
        { url: "https://example.com/", title: "Example" },
        { url: "https://example.org/", title: "Example Org" },
      ],
      activeIndex: 1,
      savedAt: 1_000,
    });
    const snapshot = readBrowserSession(WORKSPACE, 2_000);
    expect(snapshot?.tabs).toHaveLength(2);
    expect(snapshot?.activeIndex).toBe(1);
  });

  it("ignores a malformed snapshot instead of throwing", () => {
    localStorage.setItem(browserSessionKey(WORKSPACE), "{not json");
    expect(readBrowserSession(WORKSPACE)).toBeNull();
  });

  it("drops a snapshot older than a week", () => {
    saveBrowserSession(WORKSPACE, {
      tabs: [{ url: "https://example.com/", title: "Example" }],
      activeIndex: 0,
      savedAt: 1_000,
    });
    expect(readBrowserSession(WORKSPACE, 8 * 24 * 60 * 60 * 1000)).toBeNull();
    expect(localStorage.getItem(browserSessionKey(WORKSPACE))).toBeNull();
  });

  it("clears on demand", () => {
    saveBrowserSession(WORKSPACE, {
      tabs: [{ url: "https://example.com/", title: "Example" }],
      activeIndex: 0,
      savedAt: 1_000,
    });
    clearBrowserSession(WORKSPACE);
    expect(readBrowserSession(WORKSPACE)).toBeNull();
  });
});
