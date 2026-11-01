import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { setLanguage } from "@/lib/i18n";
import { createTimeline } from "@/lib/runEvents";
import { useRunEvents } from "@/lib/runStore";
import { useUi } from "@/lib/ui";
import type { RunView } from "@/lib/types";
import { api } from "@/lib/api";
import { UsagePanel } from "./UsagePanel";

vi.mock("@/lib/api", () => ({
  api: {
    listRuns: vi.fn(async () => [
      {
        id: "run-1",
        conversation_id: "conv-1",
        mode: "agent",
        status: "running",
        model_id: "p:gpt-4o",
        step_count: 3,
        tool_call_count: 4,
        failed_tool_count: 0,
        started_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        summary: "",
        budget: {},
        error: null,
        usage: null,
      } satisfies RunView,
    ]),
    listRunEvents: vi.fn(async () => []),
  },
}));

function renderPanel() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <UsagePanel embedded />
    </QueryClientProvider>,
  );
}

beforeAll(() => setLanguage("ja"));

afterEach(() => {
  useUi.setState({ conversationId: null, activityRunId: null });
  useRunEvents.setState({
    liveRunId: null,
    conversationId: null,
    timeline: null,
    byConversation: {},
  });
});

describe("UsagePanel", () => {
  it("shows live context, cost and token totals for the current run", async () => {
    useUi.setState({ conversationId: "conv-1", activityRunId: null });
    const timeline = createTimeline("run-1");
    timeline.usage = {
      input_tokens: 60_000,
      output_tokens: 1_200,
      cache_read_tokens: 30_000,
      cache_write_tokens: 1_000,
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

    // Context ring and the matching header badge both read 75%.
    expect(screen.getAllByText("75%").length).toBeGreaterThan(0);
    expect(screen.getByText("9.6万")).toBeInTheDocument();
    // The current Run and the conversation total both show the same estimate.
    expect(screen.getAllByText("$0.0012").length).toBeGreaterThan(0);
    expect(screen.getAllByText("6万").length).toBeGreaterThan(0);
    expect(await screen.findByText("gpt-4o")).toBeInTheDocument();
    expect(api.listRunEvents).not.toHaveBeenCalled();
  });
});
