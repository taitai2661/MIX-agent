import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ResizableHandle } from "./ResizableHandle";

describe("ResizableHandle", () => {
  it("supports keyboard resizing and commits the updated width", () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    render(
      <ResizableHandle
        value={260}
        min={200}
        max={320}
        label="Sidebar width"
        onChange={onChange}
        onCommit={onCommit}
      />,
    );

    fireEvent.keyDown(screen.getByRole("separator"), { key: "ArrowRight" });

    expect(onChange).toHaveBeenCalledWith(272);
    expect(onCommit).toHaveBeenCalledWith(272);
  });

  it("clamps keyboard resizing to the pane limits", () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    render(
      <ResizableHandle
        value={310}
        min={200}
        max={320}
        label="Sidebar width"
        onChange={onChange}
        onCommit={onCommit}
      />,
    );

    fireEvent.keyDown(screen.getByRole("separator"), {
      key: "ArrowRight",
      shiftKey: true,
    });

    expect(onChange).toHaveBeenCalledWith(320);
    expect(onCommit).toHaveBeenCalledWith(320);
  });

  it("updates the panel while dragging and saves its final width", () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    const capture = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "setPointerCapture", {
      configurable: true,
      value: capture,
    });
    const { unmount } = render(
      <ResizableHandle
        value={260}
        min={200}
        max={320}
        label="Sidebar width"
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    const separator = screen.getByRole("separator");

    fireEvent.pointerDown(separator, { pointerId: 1, clientX: 300, button: 0 });
    fireEvent.pointerMove(separator, { pointerId: 1, clientX: 340 });
    fireEvent.pointerUp(separator, { pointerId: 1, clientX: 340 });

    expect(capture).toHaveBeenCalledWith(1);
    expect(onChange).toHaveBeenCalledWith(300);
    expect(onCommit).toHaveBeenCalledWith(300);
    unmount();
    delete (HTMLElement.prototype as Partial<HTMLElement>).setPointerCapture;
  });
});
