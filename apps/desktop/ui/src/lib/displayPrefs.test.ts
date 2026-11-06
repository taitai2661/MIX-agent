import { describe, expect, it } from "vitest";
import { applyDisplayPrefs, clampZoom } from "./ui";

describe("clampZoom", () => {
  it("clamps to range and rounds to 0.1", () => {
    expect(clampZoom(1.234)).toBe(1.2);
    expect(clampZoom(Number.NaN)).toBe(1);
    expect(clampZoom(99)).toBeLessThanOrEqual(1.5);
    expect(clampZoom(0)).toBeGreaterThanOrEqual(0.8);
  });
});

describe("applyDisplayPrefs", () => {
  it("sets zoom property and font-size fallback", () => {
    applyDisplayPrefs({ zoom: 1.2, codeWrap: true, reduceMotion: false });
    const root = document.documentElement;
    expect(root.style.getPropertyValue("zoom")).toBe("1.2");
    expect(root.style.fontSize).toBe(`${16 * 1.2}px`);
    expect(root.dataset.codeWrap).toBe("wrap");
  });
});
