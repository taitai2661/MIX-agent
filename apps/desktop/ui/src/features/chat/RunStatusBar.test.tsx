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
) {
  const timeline = createTimeline("run-1");
  timeline.status = status;
  timeline.tools = tools;
  timeline.approval = approval;
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

  it("renders nothing once the run is terminal", () => {
    setLive("completed");
    const { container } = render(<RunStatusBar conversationId="conv-1" />);
    expect(container).toBeEmptyDOMElement();
  });
});
