import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { setLanguage } from "@/lib/i18n";
import { createTimeline } from "@/lib/runEvents";
import { useRunEvents } from "@/lib/runStore";
import { useUi } from "@/lib/ui";
import type { RunView } from "@/lib/types";
import { RightPanel } from "./RightPanel";

vi.mock("@/lib/api", () => ({
  api: {
    listRuns: vi.fn(async () => [
      {
        id: "run-1",
        conversation_id: "conv-1",
        mode: "chat",
        status: "completed",
        model_id: "p:gpt-4o",
        step_count: 1,
        tool_call_count: 0,
        failed_tool_count: 0,
        started_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        summary: "",
        budget: {},
        error: null,
        usage: null,
      } satisfies RunView,
    ]),
  },
}));

function renderPanel() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <RightPanel />
    </QueryClientProvider>,
  );
}

beforeAll(() => setLanguage("ja"));

afterEach(() => {
  useUi.setState({
    surface: "desktop-agent",
    rightPanel: "none",
    conversationId: null,
    activityRunId: null,
  });
  useRunEvents.setState({
    liveRunId: null,
    conversationId: null,
    timeline: null,
    byConversation: {},
  });
});

describe("RightPanel on the chat surface", () => {
  it("offers usage and activity but keeps agent-only tabs out", async () => {
    useUi.setState({
      surface: "chat",
      rightPanel: "usage",
      conversationId: "conv-1",
    });
    const timeline = createTimeline("run-1");
    timeline.usage = {
      input_tokens: 60_000,
      output_tokens: 1_200,
      cache_read_tokens: 0,
      cache_write_tokens: 0,
      context_tokens: 96_000,
      context_window: 128_000,
      estimated_cost_usd: 0.0012,
    };
    useRunEvents.setState({
      liveRunId: "run-1",
      conversationId: "conv-1",
      timeline,
      byConversation: { "conv-1": { runId: "run-1", timeline } },
    });

    renderPanel();

    expect(screen.getByRole("radio", { name: "使用状況" })).toBeInTheDocument();
    expect(
      screen.getByRole("radio", { name: "このRunのアクティビティ" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("radio", { name: "変更" })).toBeNull();
    expect(screen.queryByRole("radio", { name: "ブラウザ" })).toBeNull();
    expect(await screen.findByText("gpt-4o")).toBeInTheDocument();
  });
});
