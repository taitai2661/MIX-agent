import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { api, type RunEventPayload } from "@/lib/api";
import { setLanguage } from "@/lib/i18n";
import { applyRunEvent, createTimeline } from "@/lib/runEvents";
import { ApprovalCard } from "./ApprovalCard";

beforeAll(() => setLanguage("ja"));

vi.mock("@/lib/api", () => ({
  api: {
    decideApproval: vi.fn(async () => undefined),
    extendApproval: vi.fn(async ({ id }: { id: string; seconds: number }) => ({
      id,
      expires_at: new Date(Date.now() + 5 * 60_000).toISOString(),
    })),
  },
}));

function riskyTimeline(approvalId: string, command: string) {
  const event: RunEventPayload = {
    run_id: "run-1",
    sequence: 1,
    kind: "tool_approval_requested",
    data: {
      tool_call_id: `tc-${approvalId}`,
      approval_id: approvalId,
      name: "terminal",
      arguments: { command },
      expires_at: new Date(Date.now() + 60_000).toISOString(),
      risk: {
        level: "dangerous",
        reasons: [{ code: "recursive_delete", detail: command }],
      },
    },
    created_at: "2026-01-01T00:00:00Z",
  };
  return applyRunEvent(createTimeline("run-1"), event);
}

beforeEach(() => vi.clearAllMocks());

describe("ApprovalCard", () => {
  it("does not carry acknowledgement across two risky approvals in one run", async () => {
    const first = riskyTimeline("ap-1", "rm -rf /tmp/a");
    const second = riskyTimeline("ap-2", "rm -rf /");
    const { rerender } = render(<ApprovalCard live={first} />);

    const checkbox = () =>
      screen.getByRole("checkbox", {
        name: /内容を理解しました/,
      });
    const approve = () => screen.getByRole("button", { name: /今回のみ許可/ });

    expect(approve()).toBeDisabled();
    fireEvent.click(checkbox());
    expect(approve()).toBeEnabled();

    // The second approval replaces the first while the component stays mounted.
    rerender(<ApprovalCard live={second} />);
    expect(checkbox()).not.toBeChecked();
    expect(approve()).toBeDisabled();

    fireEvent.click(checkbox());
    await act(async () => {
      fireEvent.click(approve());
    });
    await vi.waitFor(() =>
      expect(api.decideApproval).toHaveBeenCalledWith({
        id: "ap-2",
        grant: true,
        remember: false,
        rememberSession: false,
        acknowledgeRisk: true,
      }),
    );
  });

  it("extends the deadline by five minutes from the card", async () => {
    const live = riskyTimeline("ap-1", "echo hello");
    render(<ApprovalCard live={live} />);

    fireEvent.pointerDown(screen.getByRole("button", { name: "+5分" }), {
      button: 0,
      ctrlKey: false,
    });
    const fiveMinutes = await screen.findByRole("menuitem", {
      name: "5分延長",
    });
    await act(async () => {
      fireEvent.click(fiveMinutes);
    });
    expect(api.extendApproval).toHaveBeenCalledWith({
      id: "ap-1",
      seconds: 300,
    });

    // The preset menu offers longer windows than the old fixed five minutes.
    fireEvent.pointerDown(screen.getByRole("button", { name: "+5分" }), {
      button: 0,
      ctrlKey: false,
    });
    const fifteenMinutes = await screen.findByRole("menuitem", {
      name: "15分延長",
    });
    await act(async () => {
      fireEvent.click(fifteenMinutes);
    });
    expect(api.extendApproval).toHaveBeenLastCalledWith({
      id: "ap-1",
      seconds: 900,
    });
  });

  it("drops acknowledgement when the same id describes a different command", async () => {
    const first = riskyTimeline("ap-1", "rm -rf /tmp/a");
    const second = riskyTimeline("ap-1", "rm -rf /");
    const { rerender } = render(<ApprovalCard live={first} />);
    const checkbox = () =>
      screen.getByRole("checkbox", { name: /内容を理解しました/ });
    const approve = () => screen.getByRole("button", { name: /今回のみ許可/ });

    fireEvent.click(checkbox());
    expect(approve()).toBeEnabled();
    rerender(<ApprovalCard live={second} />);
    expect(checkbox()).not.toBeChecked();
    expect(approve()).toBeDisabled();

    await act(async () => {
      fireEvent.click(approve());
    });
    expect(api.decideApproval).not.toHaveBeenCalled();
  });
});
