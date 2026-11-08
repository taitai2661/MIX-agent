import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { setLanguage } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { api } from "@/lib/api";
import type { PendingApprovalView } from "@/lib/types";
import { ApprovalsPane } from "./ApprovalsPane";

vi.mock("@/lib/api", () => ({
  api: {
    listPendingApprovals: vi.fn(),
    decideApproval: vi.fn(),
  },
}));

const mocked = vi.mocked(api, true);

const approval: PendingApprovalView = {
  approval_id: "approval-1",
  run_id: "run-1234567890",
  tool_call_id: "tool-1",
  tool_name: "terminal",
  arguments: { command: "npm test" },
  created_at: new Date().toISOString(),
  expires_at: new Date(Date.now() + 60_000).toISOString(),
  conversation_id: "conv-1",
  conversation_title: "テストを実行",
  conversation_mode: "agent",
};

function renderPane() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  return render(
    <QueryClientProvider client={client}>
      <ApprovalsPane location="drawer" />
    </QueryClientProvider>,
  );
}

beforeAll(() => setLanguage("ja"));

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mocked.listPendingApprovals.mockResolvedValue([approval]);
  useUi.setState({
    surface: "chat",
    conversationId: null,
    activityRunId: null,
    rightPanel: "none",
    approvalsOpen: true,
  });
});

describe("ApprovalsPane navigation", () => {
  it("opens the conversation and closes the drawer", async () => {
    const user = userEvent.setup();
    renderPane();

    await user.click(
      await screen.findByRole("button", { name: "スレッドを開く" }),
    );

    expect(useUi.getState().surface).toBe("desktop-agent");
    expect(useUi.getState().conversationId).toBe("conv-1");
    expect(useUi.getState().approvalsOpen).toBe(false);
    expect(useUi.getState().activityRunId).toBeNull();
  });

  it("can open the Run's activity for more context", async () => {
    const user = userEvent.setup();
    renderPane();

    await user.click(
      await screen.findByRole("button", { name: "アクティビティ" }),
    );

    expect(useUi.getState().conversationId).toBe("conv-1");
    expect(useUi.getState().activityRunId).toBe("run-1234567890");
    expect(useUi.getState().rightPanel).toBe("activity");
    expect(useUi.getState().approvalsOpen).toBe(false);
    await waitFor(() => expect(mocked.listPendingApprovals).toHaveBeenCalled());
  });
});
