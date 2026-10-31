import { afterEach, describe, expect, it } from "vitest";
import { act, fireEvent, renderHook } from "@testing-library/react";
import { useUi } from "@/lib/ui";
import { useAppShortcuts } from "./useShellEvents";

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
