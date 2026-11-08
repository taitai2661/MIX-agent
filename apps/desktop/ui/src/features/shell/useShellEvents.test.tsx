import { afterEach, describe, expect, it } from "vitest";
import { act, fireEvent, renderHook } from "@testing-library/react";
import { useUi } from "@/lib/ui";
import { browserSplitRatio, useAppShortcuts } from "./useShellEvents";

describe("browserSplitRatio", () => {
  it("reads the tool hint and clamps it to the supported range", () => {
    expect(browserSplitRatio({ split_ratio: 0.65 })).toBe(0.65);
    expect(browserSplitRatio({ split_ratio: "0.4" })).toBe(0.4);
    expect(browserSplitRatio({ split_ratio: 2 })).toBe(0.8);
    expect(browserSplitRatio({ split_ratio: 0 })).toBe(0.2);
  });

  it("treats an absent or invalid hint as no hint", () => {
    expect(browserSplitRatio({ action: "navigate" })).toBeNull();
    expect(browserSplitRatio({ split_ratio: "wide" })).toBeNull();
    expect(browserSplitRatio(null)).toBeNull();
  });
});

describe("global Escape handling", () => {
  afterEach(() => {
    document
      .querySelectorAll("[data-test-overlay]")
      .forEach((el) => el.remove());
    useUi.setState({
      rightPanel: "none",
      paletteOpen: false,
      shortcutsOpen: false,
    });
  });

  it("does not close the right panel when a dialog above it owns Escape", () => {
    renderHook(() => useAppShortcuts());
    useUi.setState({ rightPanel: "changes" });
    const dialog = document.createElement("div");
    dialog.dataset.testOverlay = "";
    dialog.setAttribute("role", "dialog");
    dialog.setAttribute("data-state", "open");
    document.body.append(dialog);

    act(() => {
      fireEvent.keyDown(window, { key: "Escape" });
    });
    expect(useUi.getState().rightPanel).toBe("changes");

    // Once the dialog is gone, the same key closes the panel.
    dialog.remove();
    act(() => {
      fireEvent.keyDown(window, { key: "Escape" });
    });
    expect(useUi.getState().rightPanel).toBe("none");
  });
});
