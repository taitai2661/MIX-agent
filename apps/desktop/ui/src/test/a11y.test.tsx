/// Accessibility regression checks for the approval path.
///
/// The project already runs `eslint-plugin-jsx-a11y`, but static lint cannot
/// see an `aria-describedby` that points at no element, a disabled control
/// whose reason is not announced, or an alertdialog with no accessible name.
/// `axe-core` runs in jsdom for everything except colour contrast, which needs
/// a real layout engine.
import { fireEvent, render, screen } from "@testing-library/react";
import axe from "axe-core";
import { describe, expect, it, vi } from "vitest";
import type { RunEventPayload } from "@/lib/api";
import type { MessageView } from "@/lib/types";
import { applyRunEvent, createTimeline } from "@/lib/runEvents";
import { ApprovalCard } from "@/features/chat/ApprovalCard";
import { MessageItem } from "@/features/chat/MessageItem";

vi.mock("@/lib/api", () => ({
  api: {
    decideApproval: vi.fn(async () => undefined),
  },
}));

function event(kind: string, data: unknown): RunEventPayload {
  return {
    run_id: "run-1",
    sequence: 1,
    kind,
    data,
    created_at: "2026-01-01T00:00:00Z",
  };
}

function dangerousTimeline() {
  return applyRunEvent(
    createTimeline("run-1"),
    event("tool_approval_requested", {
      tool_call_id: "tc-1",
      approval_id: "ap-1",
      name: "terminal",
      arguments: { command: "rm -rf /" },
      expires_at: new Date(Date.now() + 60_000).toISOString(),
      risk: {
        level: "dangerous",
        reasons: [{ code: "recursive_delete", detail: "rm -rf /" }],
      },
    }),
  );
}

async function expectNoAxeViolations(container: HTMLElement) {
  const results = await axe.run(container, {
    rules: {
      // jsdom has no layout, so axe cannot compute contrast or visibility.
      "color-contrast": { enabled: false },
      region: { enabled: false },
    },
  });
  expect(results.violations).toEqual([]);
}

describe("approval accessibility", () => {
  it("names the dialog, announces the risk, and gates approval on the checkbox", async () => {
    const { container } = render(<ApprovalCard live={dangerousTimeline()} />);

    const dialog = screen.getByRole("alertdialog");
    expect(dialog).toHaveAccessibleName(/承認が必要です|Approval required/);
    // The description must resolve to the diff container, not a dangling id.
    expect(dialog.getAttribute("aria-describedby")).toBeTruthy();
    expect(
      document.getElementById(dialog.getAttribute("aria-describedby")!),
    ).not.toBeNull();

    const approve = screen.getByRole("button", {
      name: /今回のみ許可|Allow once/,
    });
    expect(approve).toBeDisabled();
    const checkbox = screen.getByRole("checkbox", {
      name: /内容を理解しました|I understand/,
    });
    fireEvent.click(checkbox);
    expect(approve).toBeEnabled();

    // A dangerous command cannot be remembered, so neither button exists.
    expect(
      screen.queryByRole("button", { name: /常に許可|Always allow/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", {
        name: /このセッションで許可|Allow for this session/,
      }),
    ).not.toBeInTheDocument();

    await expectNoAxeViolations(container);
  });
});

describe("auto answer accessibility", () => {
  const assistant: MessageView = {
    id: "m-auto",
    role: "assistant",
    content: "Auto が選んだ回答です。",
    reasoning: null,
    model_id: "prov:model-x",
    run_id: null,
    kind: "assistant",
    created_at: "2026-01-01T00:00:00Z",
    attachments: [],
    seq: 1,
    auto_selection: true,
    auto_reason: "Auto: balanced の実績から選択",
    feedback: "up",
  };

  it("labels the Auto badge and the rating buttons", async () => {
    const { container } = render(
      <MessageItem message={assistant} runs={new Map()} now={Date.now()} />,
    );

    // The badge's accessible name includes the routing reason, so a screen
    // reader hears why the model was chosen instead of a bare "Auto".
    expect(
      screen
        .getByText("Auto")
        .closest("[aria-label]")
        ?.getAttribute("aria-label"),
    ).toContain("balanced");
    const up = screen.getByRole("button", {
      name: /高く評価|rate this answer highly/i,
    });
    expect(up).toHaveAttribute("aria-pressed", "true");

    await expectNoAxeViolations(container);
  });
});
