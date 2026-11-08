/// Smoke coverage for the sidebar shell.
///
/// The sidebar is the only way to reach history, projects and saved agents,
/// yet it had no render test at all. These cases pin the surface labels, the
/// empty state and the navigation wiring so a layout or store regression fails
/// here instead of only in a packaged build.

import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, cleanup } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach } from "vitest";
import { setLanguage } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { Sidebar } from "./Sidebar";

vi.mock("@/lib/queries", () => ({
  CONVERSATIONS_QUERY_KEY: ["conversations"],
  useConversationPages: () => ({
    data: { pages: [{ items: [], has_more: false }] },
    hasNextPage: false,
    isFetchingNextPage: false,
    isPending: false,
    fetchNextPage: vi.fn(),
  }),
  useConversationSearch: () => ({
    data: [],
    isPending: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
  useProjects: () => ({ data: [], isPending: false, isError: false }),
}));

vi.mock("@/lib/settings", () => ({
  useApplySettings: () => vi.fn(async () => undefined),
  useSettingsForm: () => ({ dirty: false }),
}));

vi.mock("@/lib/api", () => ({
  api: {
    renameConversation: vi.fn(),
    deleteConversation: vi.fn(),
    pickSaveFile: vi.fn(),
    exportMarkdown: vi.fn(),
    saveTextFile: vi.fn(),
  },
}));

vi.mock("./useShellEvents", () => ({
  useNewThread: () => vi.fn(),
}));

vi.mock("@/components/ui/dialog", () => ({
  useConfirm: () => ({ confirm: vi.fn(async () => false), dialog: null }),
}));

beforeAll(() => setLanguage("ja"));

beforeEach(() => {
  useUi.setState({
    view: "chat",
    surface: "chat",
    conversationId: null,
    activeProjectId: null,
    sidebarOpen: true,
    locale: "ja",
    theme: "system",
  });
});

afterEach(() => cleanup());

function renderSidebar() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <Sidebar />
    </QueryClientProvider>,
  );
}

describe("Sidebar", () => {
  it("labels the history for the active surface and shows the empty state", () => {
    renderSidebar();

    expect(screen.getByText("チャットの履歴")).toBeInTheDocument();
    expect(screen.getByText("新しい会話を始めましょう")).toBeInTheDocument();
    expect(
      screen.getByPlaceholderText("チャットの履歴を検索"),
    ).toBeInTheDocument();
  });

  it("switches the history label with the coding surface", () => {
    useUi.setState({ surface: "desktop-agent", view: "chat" });
    renderSidebar();

    expect(screen.getByText("コーディングの履歴")).toBeInTheDocument();
    expect(
      screen.getByText("新しいコーディングを始めましょう"),
    ).toBeInTheDocument();
  });

  it("navigates to projects and agent settings from the grouped nav", () => {
    renderSidebar();

    fireEvent.click(screen.getByText("プロジェクト"));
    expect(useUi.getState().view).toBe("projects");

    fireEvent.click(screen.getByText("エージェント設定"));
    expect(useUi.getState().view).toBe("agents");

    fireEvent.click(screen.getByText("スケジュール"));
    expect(useUi.getState().view).toBe("schedules");
  });
});
