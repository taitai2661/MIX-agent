import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { usePanelWidth } from "./usePanelWidth";

describe("usePanelWidth", () => {
  beforeEach(() => localStorage.clear());

  it("restores a committed width on the next mount", () => {
    const { result, unmount } = renderHook(() => usePanelWidth("sidebar-width", 264, 210, 440));
    act(() => {
      result.current.setWidth(320);
      result.current.commitWidth(320);
    });
    unmount();

    const restored = renderHook(() => usePanelWidth("sidebar-width", 264, 210, 440));
    expect(restored.result.current.width).toBe(320);
    restored.unmount();
  });

  it("ignores saved widths outside the panel limits", () => {
    localStorage.setItem("sidebar-width", "900");
    const { result } = renderHook(() => usePanelWidth("sidebar-width", 264, 210, 440));
    expect(result.current.width).toBe(264);
  });
});
