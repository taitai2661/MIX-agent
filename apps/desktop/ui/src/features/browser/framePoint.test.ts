import { describe, expect, it } from "vitest";
import { framePoint } from "./framePoint";

const PAGE = { width: 1280, height: 900 };

describe("framePoint", () => {
  it("maps the centre of an exactly-fitted image to the page centre", () => {
    const point = framePoint(
      640,
      450,
      { left: 0, top: 0, width: 1280, height: 900 },
      { width: 1280, height: 900 },
      PAGE,
    );
    expect(point).toEqual({ x: 640, y: 450 });
  });

  it("accounts for letterboxing instead of using the element box", () => {
    // The box is 1600x1200 but the frame is 1600x900, so the drawn image sits
    // at y=150..1050. A click in the top 150 px is outside the picture and must
    // be ignored, not mapped to y=0.
    const rect = { left: 0, top: 0, width: 1600, height: 1200 };
    const natural = { width: 1600, height: 900 };
    expect(framePoint(800, 600, rect, natural, PAGE)).toEqual({
      x: 640,
      y: 450,
    });
    expect(framePoint(800, 10, rect, natural, PAGE)).toBeNull();
  });

  it("scales a downscaled frame back to CSS pixels", () => {
    const point = framePoint(
      50,
      50,
      { left: 10, top: 20, width: 640, height: 450 },
      { width: 1280, height: 900 },
      PAGE,
    );
    expect(point).toEqual({ x: 80, y: 60 });
  });

  it("returns null for an empty element", () => {
    expect(
      framePoint(
        0,
        0,
        { left: 0, top: 0, width: 0, height: 0 },
        { width: 1280, height: 900 },
        PAGE,
      ),
    ).toBeNull();
  });
});
