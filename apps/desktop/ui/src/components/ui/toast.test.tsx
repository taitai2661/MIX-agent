import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useToasts } from "@/lib/toast";
import { Toaster } from "./toast";

afterEach(() => {
  useToasts.setState({ toasts: [] });
});

describe("Toaster", () => {
  it("runs the undo action and dismisses the toast", async () => {
    const user = userEvent.setup();
    const undo = vi.fn();
    const onDismiss = vi.fn();
    render(<Toaster />);

    act(() => {
      useToasts.getState().push({
        message: "「Thread」を削除しました",
        action: { label: "削除を取り消す", onClick: undo },
        timeoutMs: 7000,
        onDismiss,
      });
    });

    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("Thread");

    await user.click(screen.getByRole("button", { name: "削除を取り消す" }));
    expect(undo).toHaveBeenCalledTimes(1);
    // Undo cancelled the deferred work.
    expect(onDismiss).not.toHaveBeenCalled();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("runs onDismiss when the toast times out or is closed", () => {
    vi.useFakeTimers();
    try {
      const onDismiss = vi.fn();
      render(<Toaster />);
      act(() => {
        useToasts
          .getState()
          .push({ message: "bye", timeoutMs: 1000, onDismiss });
      });
      expect(screen.getByRole("status")).toBeInTheDocument();

      act(() => {
        vi.advanceTimersByTime(1001);
      });
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
      expect(onDismiss).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("restarts the timer when a fixed id is re-pushed and settles the old toast", () => {
    vi.useFakeTimers();
    try {
      const firstDismiss = vi.fn();
      render(<Toaster />);
      act(() => {
        useToasts.getState().push({
          id: "autosave",
          message: "first",
          timeoutMs: 1000,
          onDismiss: firstDismiss,
        });
      });
      act(() => {
        vi.advanceTimersByTime(600);
      });
      act(() => {
        useToasts.getState().push({
          id: "autosave",
          message: "second",
          timeoutMs: 1000,
        });
      });
      // The replaced toast's deferred work is no longer cancellable, so it
      // must run instead of being silently dropped.
      expect(firstDismiss).toHaveBeenCalledTimes(1);
      act(() => {
        vi.advanceTimersByTime(600);
      });
      // The remaining 400ms of the old timer must not dismiss the new toast.
      expect(screen.getByRole("status")).toHaveTextContent("second");
      act(() => {
        vi.advanceTimersByTime(400);
      });
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not let a throwing onDismiss break dismissal", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    render(<Toaster />);
    act(() => {
      useToasts.getState().push({
        message: "boom",
        timeoutMs: 10,
        onDismiss: () => {
          throw new Error("nope");
        },
      });
    });
    const id = useToasts.getState().toasts[0]!.id;
    expect(() => useToasts.getState().dismiss(id)).not.toThrow();
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });

  it("pauses while hovered so the undo window is reachable", () => {
    vi.useFakeTimers();
    try {
      const onDismiss = vi.fn();
      render(<Toaster />);
      act(() => {
        useToasts
          .getState()
          .push({ message: "hold", timeoutMs: 1000, onDismiss });
      });
      const row = screen.getByRole("status").parentElement!;

      fireEvent.pointerEnter(row);
      act(() => {
        vi.advanceTimersByTime(5000);
      });
      expect(screen.getByRole("status")).toBeInTheDocument();
      expect(onDismiss).not.toHaveBeenCalled();

      fireEvent.pointerLeave(row);
      act(() => {
        vi.advanceTimersByTime(1001);
      });
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
      expect(onDismiss).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
