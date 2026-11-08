import { describe, expect, it } from "vitest";
import { tabAvatar } from "./BrowserTabStrip";

describe("tabAvatar", () => {
  it("uses the host initial and a stable tone", () => {
    const first = tabAvatar("https://www.example.com/a");
    const second = tabAvatar("https://www.example.com/b");
    expect(first.label).toBe("E");
    expect(first.tone).toBe(second.tone);
  });

  it("has no avatar for a blank tab", () => {
    expect(tabAvatar("about:blank")).toEqual({ label: "", tone: "" });
    expect(tabAvatar(null)).toEqual({ label: "", tone: "" });
  });

  it("survives a malformed URL", () => {
    expect(tabAvatar("not a url").label).toBe("N");
  });
});
