import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { setLanguage } from "@/lib/i18n";
import { ApprovalCountdown } from "./ApprovalCountdown";

beforeAll(() => setLanguage("ja"));

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("ApprovalCountdown", () => {
  it("shares one timer between every countdown on screen", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
    const expiresAt = new Date(Date.now() + 61_000).toISOString();

    const view = render(
      <>
        <ApprovalCountdown expiresAt={expiresAt} />
        <ApprovalCountdown expiresAt={expiresAt} />
      </>,
    );

    // Observable behaviour rather than how the timer is created: two cards
    // share a single live ticker, and one tick updates both labels together.
    expect(vi.getTimerCount()).toBe(1);
    expect(screen.getAllByText("残り 1:01")).toHaveLength(2);
    act(() => {
      vi.advanceTimersByTime(1_000);
    });
    expect(screen.getAllByText("残り 1:00")).toHaveLength(2);

    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(screen.getAllByText("期限切れ（拒否されます）")).toHaveLength(2);
    // The shared ticker stops as soon as no deadline is in the future.
    expect(vi.getTimerCount()).toBe(0);

    view.unmount();
  });
});
