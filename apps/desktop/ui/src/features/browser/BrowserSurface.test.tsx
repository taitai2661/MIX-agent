import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";
import { BrowserSurface } from "./BrowserSurface";

describe("BrowserSurface", () => {
  it("reports the same-sized rectangle again when the bounds callback changes", () => {
    // A workspace switch hands the surface a new callback. The rectangle can
    // be identical to the previous thread's, so reporting must not be
    // deduplicated against the old surface or the new native WebView stays
    // hidden.
    const rect = {
      x: 10,
      y: 20,
      width: 640,
      height: 480,
      top: 20,
      right: 650,
      bottom: 500,
      left: 10,
      toJSON: () => ({}),
    } as DOMRect;
    const spy = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockReturnValue(rect);

    const first = vi.fn();
    const { rerender } = render(<BrowserSurface setBounds={first} />);
    expect(first).toHaveBeenCalledWith({
      x: 10,
      y: 20,
      width: 640,
      height: 480,
      visible: true,
    });

    const second = vi.fn();
    rerender(<BrowserSurface setBounds={second} />);

    expect(second).toHaveBeenCalledWith({
      x: 10,
      y: 20,
      width: 640,
      height: 480,
      visible: true,
    });

    spy.mockRestore();
  });
});
