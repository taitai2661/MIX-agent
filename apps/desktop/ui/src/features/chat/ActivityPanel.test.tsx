import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  cleanup,
} from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { setLanguage } from "@/lib/i18n";
import {
  createTimeline,
  type ContextCompaction,
  type TodoItem,
  type ToolActivity,
} from "@/lib/runEvents";
import { useRunEvents } from "@/lib/runStore";
import { useUi } from "@/lib/ui";
import { api } from "@/lib/api";
import { ActivityPanel } from "./ActivityPanel";

vi.mock("@/lib/api", () => ({
  api: {
    listRunEvents: vi.fn(async () => []),
    runWorkspaceChanges: vi.fn(async () => ({ available: false, files: [] })),
    listCheckpoints: vi.fn(async () => []),
    revertRunWorkspace: vi.fn(async () => 2),
    resumeRun: vi.fn(async () => "run-9"),
  },
}));

function renderPanel({ embedded = false }: { embedded?: boolean } = {}) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ActivityPanel embedded={embedded} />
    </QueryClientProvider>,
  );
}

function liveTimeline(
  tools: ToolActivity[],
  extras: Partial<ReturnType<typeof createTimeline>> = {},
) {
  const timeline = createTimeline("run-1");
  timeline.tools = tools;
  return Object.assign(timeline, extras);
}

beforeAll(() => setLanguage("ja"));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  useUi.setState({
    conversationId: null,
    activityRunId: null,
    rightPanel: "none",
    diffTarget: null,
  });
  useRunEvents.setState({
    liveRunId: null,
    conversationId: null,
    timeline: null,
    byConversation: {},
  });
});

const startedAt = "2026-01-01T00:00:00.000Z";
const finishedAt = "2026-01-01T00:00:01.500Z";

describe("ActivityPanel", () => {
  it("renders live tool calls with every status, plan and verification notice", () => {
    const tools: ToolActivity[] = [
      {
        id: "t1",
        name: "terminal",
        arguments: { command: "pnpm test", args: ["--run"] },
        status: "done",
        result: { stdout: "ok", exit_code: 0 },
        startedAt,
        finishedAt,
      },
      {
        id: "t2",
        name: "files",
        arguments: { action: "write", path: "src/a.ts", content: "hello" },
        status: "done",
        result: "written",
        startedAt,
        finishedAt,
      },
      {
        id: "t3",
        name: "files",
        arguments: { action: "write", path: "src/b.ts" },
        status: "running",
      },
      {
        id: "t4",
        name: "edit",
        arguments: { path: "src/c.ts", old_string: "a", new_string: "b" },
        status: "error",
        error: "edit failed",
        result: "@@ line 1 @@",
      },
      {
        id: "t5",
        name: "edit",
        arguments: { path: "src/d.ts", old_string: 42, new_string: "b" },
        status: "done",
      },
      {
        id: "t6",
        name: "todo",
        arguments: null,
        status: "done",
      },
      {
        id: "t7",
        name: "user_plan",
        arguments: { plan: "step" },
        status: "done",
      },
      {
        id: "t8",
        name: "lsp",
        arguments: { symbol: "Foo" },
        status: "done",
      },
      {
        id: "t9",
        name: "web_search",
        arguments: { query: "docs" },
        status: "done",
        result: "result text",
      },
    ];
    const todos: TodoItem[] = [
      { id: 1, content: "done step", status: "completed" },
      { id: 2, content: "active step", status: "in_progress" },
      { id: 3, content: "pending step", status: "pending" },
    ];
    const compaction: ContextCompaction = {
      at: finishedAt,
      compressed: true,
      activeChars: 1200,
      archiveRows: 3,
      messages: 4,
      sourceMessages: 9,
      evictedToolResults: 2,
    };
    const timeline = liveTimeline(tools, {
      todos,
      contextCompactions: [compaction],
      verificationFailed: "未確認の変更があります",
    });

    useUi.setState({ conversationId: "conv-1", activityRunId: null });
    useRunEvents.setState({
      liveRunId: "run-1",
      conversationId: "conv-1",
      timeline,
      byConversation: { "conv-1": { runId: "run-1", timeline } },
    });

    renderPanel();

    // Header carries the run id and closes both the activity scope and panel.
    expect(screen.getByText("このRunのアクティビティ")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "閉じる" }));
    expect(useUi.getState().activityRunId).toBeNull();
    expect(useUi.getState().rightPanel).toBe("none");

    expect(screen.getByText("計画")).toBeInTheDocument();
    expect(screen.getByText("done step")).toBeInTheDocument();
    expect(screen.getByText("active step")).toBeInTheDocument();
    expect(screen.getByText("pending step")).toBeInTheDocument();
    expect(screen.getByText("履歴を圧縮しました")).toBeInTheDocument();
    expect(screen.getByText("未確認の変更があります")).toBeInTheDocument();
    expect(screen.getByText("edit failed")).toBeInTheDocument();

    // Write previews open the diff pane; a non-string path or missing content
    // suppresses the button instead of showing "undefined".
    fireEvent.click(screen.getByRole("button", { name: /src\/a\.ts/ }));
    expect(useUi.getState().diffTarget).toMatchObject({
      path: "src/a.ts",
      content: "hello",
    });
    fireEvent.click(screen.getByRole("button", { name: /src\/c\.ts/ }));
    expect(useUi.getState().diffTarget).toMatchObject({
      path: "src/c.ts",
      oldString: "a",
      newString: "b",
    });
    expect(screen.queryByRole("button", { name: /src\/b\.ts/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /src\/d\.ts/ })).toBeNull();

    // An object terminal result renders the dedicated terminal view.
    expect(screen.getAllByText("結果").length).toBeGreaterThan(0);
  });

  it("shows an empty state without a run and keeps the header out when embedded", () => {
    useUi.setState({ conversationId: null, activityRunId: null });

    const { rerender } = renderPanel();
    expect(screen.getByText("このRunのアクティビティ")).toBeInTheDocument();
    expect(screen.getByText("アクティビティはありません")).toBeInTheDocument();

    rerender(
      <QueryClientProvider client={new QueryClient()}>
        <ActivityPanel embedded />
      </QueryClientProvider>,
    );
    expect(screen.queryByText("このRunのアクティビティ")).toBeNull();
    expect(screen.getByText("アクティビティはありません")).toBeInTheDocument();
  });

  it("shows a verification card while changes are still being checked", () => {
    const timeline = liveTimeline([], {
      verification: {
        tool: "terminal",
        reason: "テスト結果を確認しています",
        outstanding: ["pnpm test"],
      },
    });
    useUi.setState({ conversationId: "conv-1", activityRunId: null });
    useRunEvents.setState({
      liveRunId: "run-1",
      conversationId: "conv-1",
      timeline,
      byConversation: { "conv-1": { runId: "run-1", timeline } },
    });

    renderPanel({ embedded: true });

    expect(screen.getByText("変更の確認中")).toBeInTheDocument();
    expect(screen.getByText("テスト結果を確認しています")).toBeInTheDocument();
  });

  it("replays a finished run with checkpoints and reverts its workspace", async () => {
    vi.mocked(api.listCheckpoints).mockResolvedValueOnce([
      { id: "cp-1", step_index: 3, created_at: finishedAt },
    ]);
    vi.mocked(api.runWorkspaceChanges).mockResolvedValueOnce({
      available: true,
      files: [{ status: "M", path: "src/a.ts" }],
    });

    useUi.setState({ conversationId: null, activityRunId: "run-9" });
    renderPanel();

    expect(await screen.findByText("チェックポイント")).toBeInTheDocument();
    expect(screen.getByText(/ステップ/)).toBeInTheDocument();
    expect(await screen.findByText("1 件の変更を確認")).toBeInTheDocument();
    expect(screen.getByText("src/a.ts")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /再開/ }));
    await waitFor(() =>
      expect(api.resumeRun).toHaveBeenCalledWith({
        id: "run-9",
        from_checkpoint: "cp-1",
        unknown_actions: [],
      }),
    );
    expect(useRunEvents.getState().liveRunId).toBe("run-9");

    fireEvent.click(screen.getByRole("button", { name: /実行前の状態に戻す/ }));
    fireEvent.click(await screen.findByRole("button", { name: "取り消す" }));
    await waitFor(() =>
      expect(api.revertRunWorkspace).toHaveBeenCalledWith("run-9"),
    );
    expect(
      await screen.findByText("2 件のファイルを実行前の内容に戻しました。"),
    ).toBeInTheDocument();
  });

  it("surfaces a history read failure instead of an empty panel", async () => {
    vi.mocked(api.listRunEvents).mockRejectedValueOnce(
      new Error("履歴を読めません"),
    );
    useUi.setState({ conversationId: null, activityRunId: "run-bad" });

    renderPanel();

    expect(await screen.findByText(/履歴を読めません/)).toBeInTheDocument();
  });

  it("reports a failed checkpoint resume", async () => {
    vi.mocked(api.listCheckpoints).mockResolvedValueOnce([
      { id: "cp-1", step_index: 1, created_at: finishedAt },
    ]);
    vi.mocked(api.resumeRun).mockRejectedValueOnce(new Error("再開できません"));
    useUi.setState({ conversationId: null, activityRunId: "run-9" });

    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: /再開/ }));

    expect(await screen.findByText("再開できません")).toBeInTheDocument();
  });
});
