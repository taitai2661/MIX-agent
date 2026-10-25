/// Send / edit / retry coverage for `ChatPane` (`features/chat/ChatPane.tsx`).
///
/// The pane wires the composer to the run mutations; these tests pin that a
/// typed send reaches `sendMessage`, an inline edit reaches `editMessage` and
/// adopts the new run, and the failure alert's retry re-edits the last user
/// turn. The Tauri API surface is mocked; the real react-query cache, ui
/// store and run store drive the flow.

import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { setLanguage } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { useRunEvents } from "@/lib/runStore";
import { api } from "@/lib/api";
import type { MessageView } from "@/lib/types";
import { ChatPane } from "./ChatPane";

vi.mock("@/lib/api", () => ({
  api: {
    listMessages: vi.fn(),
    listRuns: vi.fn(),
    listConversations: vi.fn(),
    listModels: vi.fn(),
    listProviders: vi.fn(),
    listProjects: vi.fn(),
    createConversation: vi.fn(),
    sendMessage: vi.fn(),
    editMessage: vi.fn(),
    cancelRun: vi.fn(),
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

const mocked = vi.mocked(api, true);

function userMessage(
  id: string,
  content: string,
  runId: string | null = null,
): MessageView {
  return {
    id,
    role: "user",
    content,
    reasoning: null,
    model_id: "auto",
    run_id: runId,
    kind: "user",
    created_at: new Date().toISOString(),
    attachments: [],
  };
}

function runView(id: string, error: string | null = null) {
  return {
    id,
    conversation_id: "conv-1",
    mode: "chat",
    status: error ? "failed" : "done",
    model_id: "auto",
    step_count: 1,
    tool_call_count: 0,
    started_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    summary: "",
    budget: null,
    error,
  };
}

function renderPane() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ChatPane />
    </QueryClientProvider>,
  );
}

// jsdom reports en-US; pin the language so the labels are deterministic.
beforeAll(() => setLanguage("ja"));

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  useUi.setState({
    conversationId: "conv-1",
    modelId: "auto",
    mode: "chat",
    surface: "chat",
    activeProjectId: null,
  });
  useRunEvents.getState().clear();
  mocked.listMessages.mockResolvedValue([]);
  mocked.listRuns.mockResolvedValue([]);
  mocked.listConversations.mockResolvedValue([]);
  mocked.listModels.mockResolvedValue([]);
  mocked.listProviders.mockResolvedValue([]);
  mocked.listProjects.mockResolvedValue([]);
});

describe("ChatPane send/edit/retry", () => {
  it("sends the composer draft through sendMessage and adopts the run", async () => {
    mocked.sendMessage.mockResolvedValue({ run_id: "run-1" });
    const user = userEvent.setup();
    renderPane();

    const box = await screen.findByPlaceholderText("MIX-agent に話しかける…");
    await user.type(box, "こんにちは{Enter}");

    await waitFor(() =>
      expect(mocked.sendMessage).toHaveBeenCalledWith(
        expect.objectContaining({
          conversation_id: "conv-1",
          content: "こんにちは",
          model_id: "auto",
          mode: "chat",
        }),
      ),
    );
    expect(useRunEvents.getState().liveRunId).toBe("run-1");
  });

  it("edits a user turn through editMessage and adopts the new run", async () => {
    mocked.listMessages.mockResolvedValue([
      userMessage("m-1", "元のメッセージ"),
    ]);
    mocked.editMessage.mockResolvedValue({ run_id: "run-2" });
    const user = userEvent.setup();
    renderPane();

    await user.click(
      await screen.findByRole("button", { name: "メッセージを編集" }),
    );
    const box = screen.getByRole("textbox", { name: "メッセージを編集" });
    await user.clear(box);
    await user.type(box, "直したメッセージ");
    await user.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() =>
      expect(mocked.editMessage).toHaveBeenCalledWith(
        expect.objectContaining({
          message_id: "m-1",
          content: "直したメッセージ",
        }),
      ),
    );
    expect(useRunEvents.getState().liveRunId).toBe("run-2");
  });

  it("retries a failed run by re-editing the last user turn", async () => {
    mocked.listMessages.mockResolvedValue([
      userMessage("m-1", "失敗した質問", "run-9"),
    ]);
    mocked.listRuns.mockResolvedValue([runView("run-9", "provider exploded")]);
    mocked.editMessage.mockResolvedValue({ run_id: "run-10" });
    const user = userEvent.setup();
    renderPane();

    // The orphan run error renders under the user turn that started it.
    const retry = await screen.findByRole("button", { name: "再試行" });
    await user.click(retry);

    await waitFor(() =>
      expect(mocked.editMessage).toHaveBeenCalledWith(
        expect.objectContaining({ message_id: "m-1", content: "失敗した質問" }),
      ),
    );
  });
});
