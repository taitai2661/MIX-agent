import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { setLanguage } from "@/lib/i18n";
import { createTimeline } from "@/lib/runEvents";
import { useRunEvents } from "@/lib/runStore";
import type { ToolActivity } from "@/lib/runEvents";
import { RunStatusBar } from "./RunStatusBar";

beforeAll(() => setLanguage("ja"));

afterEach(() => {
  useRunEvents.setState({
    liveRunId: null,
    conversationId: null,
    timeline: null,
    byConversation: {},
    busyConversationIds: [],
  });
});

function setLive(
  status: string,
  tools: ToolActivity[] = [],
  approval: ReturnType<typeof createTimeline>["approval"] = null,
  extra: Partial<ReturnType<typeof createTimeline>> = {},
) {
  const timeline = createTimeline("run-1");
  Object.assign(timeline, { status, tools, approval, ...extra });
  act(() => {
    useRunEvents.setState({
      liveRunId: "run-1",
      conversationId: "conv-1",
      byConversation: { "conv-1": { runId: "run-1", timeline } },
    });
  });
}

describe("RunStatusBar", () => {
  it("shows the tool currently running", () => {
    setLive("running", [
      {
        id: "tool-1",
        name: "terminal",
        arguments: { command: "npm test" },
        status: "running",
      },
    ]);
    render(<RunStatusBar conversationId="conv-1" />);
    expect(screen.getByText("terminal を実行中…")).toBeInTheDocument();
  });

  it("shows the approval state instead of the tool", () => {
    setLive(
      "waiting_approval",
      [
        {
          id: "tool-1",
          name: "terminal",
          arguments: { command: "npm test" },
          status: "running",
        },
      ],
      {
        approvalId: "approval-1",
        toolCallId: "tool-1",
        name: "terminal",
        arguments: {},
      },
    );
    render(<RunStatusBar conversationId="conv-1" />);
    expect(screen.getByText("承認待ち")).toBeInTheDocument();
  });

  it("distinguishes thinking, answering, and budget waits", () => {
    setLive("running", [], null, { reasoning: "考えている…" });
    const { rerender } = render(<RunStatusBar conversationId="conv-1" />);
    expect(screen.getByText("考え中…")).toBeInTheDocument();

    setLive("running", [], null, { reasoning: "考えている…", text: "回答" });
    rerender(<RunStatusBar conversationId="conv-1" />);
    expect(screen.getByText("応答を生成中…")).toBeInTheDocument();

    setLive("budget_extension_pending", [], null, {
      budgetRequest: { reason: "上限に達しました" },
    });
    rerender(<RunStatusBar conversationId="conv-1" />);
    expect(screen.getByText("予算延長待ち")).toBeInTheDocument();
  });

  it("shows approval wait even before the approval payload arrives", () => {
    setLive("waiting_approval");
    render(<RunStatusBar conversationId="conv-1" />);
    expect(screen.getByText("承認待ち")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "承認" })).toBeInTheDocument();
  });

  it("uses the persisted start time for the elapsed label", () => {
    setLive("running");
    render(
      <RunStatusBar
        conversationId="conv-1"
        startedAt={new Date(Date.now() - 5 * 60_000).toISOString()}
      />,
    );
    // A locally observed start would render 0:00 here; the persisted time
    // already carries the five minutes the run spent in another view.
    expect(screen.getByText(/^5:0[0-5]$/)).toBeInTheDocument();
  });

  it("renders nothing once the run is terminal", () => {
    setLive("completed");
    const { container } = render(<RunStatusBar conversationId="conv-1" />);
    expect(container).toBeEmptyDOMElement();
  });
});
