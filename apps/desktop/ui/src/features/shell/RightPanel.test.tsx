import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
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
    listModels: vi.fn(async () => [
      {
        id: "p:gpt-4o",
        provider_id: "p",
        label: "GPT-4o",
        context_window: 128_000,
        supports_vision: true,
        supports_tool_calling: true,
        supports_reasoning: false,
        auto_candidate: true,
      },
    ]),
    listProviders: vi.fn(async () => [
      {
        id: "p",
        kind: "openai",
        label: "OpenAI",
        base_url: null,
        enabled: true,
        has_api_key: true,
        preset: null,
      },
    ]),
    getConversation: vi.fn(async () => null),
    browserInvoke: vi.fn(async () => ({
      ok: true,
      payload: {
        active: { id: "t1", url: "about:blank", title: null },
        pages: [{ id: "t1", url: "about:blank", title: null, active: true }],
        can_go_back: false,
        can_go_forward: false,
      },
      artifact_id: null,
      error: null,
    })),
    browserSurfaceBounds: vi.fn(async () => undefined),
    browserStateEvent: vi.fn(async () => () => undefined),
    browserStop: vi.fn(async () => undefined),
    browserClearData: vi.fn(async () => undefined),
    openExternal: vi.fn(async () => undefined),
  },
}));

function renderPanel(
  props: {
    rightMax?: number | null;
    rowWidth?: number;
  } = {},
) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <RightPanel rightMax={props.rightMax} rowWidth={props.rowWidth} />
    </QueryClientProvider>,
  );
}

beforeAll(() => setLanguage("ja"));

afterEach(() => {
  useUi.setState({
    surface: "desktop-agent",
    rightPanel: "none",
    rightPanelDocked: true,
    rightPanelWidth: 380,
    rightPanelRatio: null,
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
    expect((await screen.findAllByText("gpt-4o")).length).toBeGreaterThan(0);
  });

  it("resizes the docked panel by dragging its left edge", () => {
    useUi.setState({
      surface: "chat",
      rightPanel: "usage",
      conversationId: "conv-1",
      rightPanelDocked: true,
      rightPanelWidth: 380,
    });
    Object.defineProperty(HTMLElement.prototype, "setPointerCapture", {
      configurable: true,
      value: vi.fn(),
    });

    try {
      renderPanel();
      const separator = screen.getByRole("separator", {
        name: "サイドパネルの幅を変更",
      });

      fireEvent.pointerDown(separator, {
        pointerId: 1,
        clientX: 600,
        button: 0,
      });
      fireEvent.pointerMove(separator, { pointerId: 1, clientX: 520 });
      fireEvent.pointerUp(separator, { pointerId: 1, clientX: 520 });

      // Dragging the right panel's left edge leftwards grows the pane.
      expect(useUi.getState().rightPanelWidth).toBe(460);
    } finally {
      delete (HTMLElement.prototype as Partial<HTMLElement>).setPointerCapture;
    }
  });

  it("uses the browser ratio and records a new one after a drag", () => {
    useUi.setState({
      surface: "desktop-agent",
      rightPanel: "browser",
      workspaceId: "11111111-1111-4111-8111-111111111111",
      rightPanelDocked: true,
      rightPanelRatio: 0.5,
      rightPanelWidth: 380,
    });
    Object.defineProperty(HTMLElement.prototype, "setPointerCapture", {
      configurable: true,
      value: vi.fn(),
    });

    try {
      const { container } = renderPanel({ rightMax: 900, rowWidth: 1000 });
      const aside = container.querySelector<HTMLElement>(
        '[data-side-panel="right"]',
      );
      expect(aside?.style.width).toBe("500px");

      const separator = screen.getByRole("separator", {
        name: "サイドパネルの幅を変更",
      });
      expect(separator.getAttribute("aria-valuemax")).toBe("900");

      fireEvent.pointerDown(separator, {
        pointerId: 1,
        clientX: 600,
        button: 0,
      });
      fireEvent.pointerMove(separator, { pointerId: 1, clientX: 520 });
      fireEvent.pointerUp(separator, { pointerId: 1, clientX: 520 });

      // 500 + 80 = 580px, remembered as 58% of the 1000px shell.
      expect(useUi.getState().rightPanelWidth).toBe(580);
      expect(useUi.getState().rightPanelRatio).toBeCloseTo(0.58, 5);
    } finally {
      delete (HTMLElement.prototype as Partial<HTMLElement>).setPointerCapture;
    }
  });
});
