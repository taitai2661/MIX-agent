import { beforeAll, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { setLanguage } from "@/lib/i18n";
import { api } from "@/lib/api";
import type { LocalRuntimeSnapshotView } from "@/lib/types";
import { LocalRuntimePanel } from "./LocalRuntimePanel";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

vi.mock("@/lib/api", () => {
  const online: LocalRuntimeSnapshotView = {
    slug: "lmstudio",
    label: "LM Studio",
    kind: "lmstudio",
    state: "online",
    online: true,
    installed: true,
    managed: true,
    endpoint: "http://127.0.0.1:1234/v1",
    version: "0.3.5",
    executable: "/Users/me/.lmstudio/bin/lms",
    install_hint: "/Applications/LM Studio.app",
    launcher: null,
    error: null,
    models: [
      {
        id: "qwen/qwen3-8b",
        label: "qwen/qwen3-8b",
        context_window: 40960,
        parameter_size: "8B",
        quantization: "4bit",
        size_bytes: 4_623_961_222,
        family: "qwen3",
        loaded: false,
        model_type: "llm",
        capabilities: {
          chat: true,
          tools: true,
          vision: false,
          reasoning: null,
          embedding: false,
        },
      },
    ],
    can_start: true,
    can_stop: true,
    can_open_app: true,
    can_pull: true,
    can_delete: false,
    can_load: true,
    can_unload: true,
    auto_start_suppressed: false,
  };
  const stopped: LocalRuntimeSnapshotView = {
    ...online,
    slug: "ollama",
    label: "Ollama",
    kind: "ollama",
    state: "stopped",
    online: false,
    managed: false,
    endpoint: "http://127.0.0.1:11434/v1",
    executable: "/opt/homebrew/bin/ollama",
    install_hint: null,
    models: [],
    can_delete: true,
  };
  return {
    api: {
      localRuntimeSnapshot: vi.fn(async () => [online, stopped]),
      localRuntimeRefresh: vi.fn(async () => [online]),
      localJobList: vi.fn(async () => []),
      localRuntimeStart: vi.fn(async () => ({ ...stopped, state: "starting" })),
      localRuntimeStop: vi.fn(),
      localRuntimeRestart: vi.fn(),
      localRuntimeOpenApp: vi.fn(),
      localRuntimeLogs: vi.fn(async () => ["listening on 127.0.0.1:1234"]),
      localModelPull: vi.fn(async () => "job-1"),
      localModelDelete: vi.fn(),
      localModelLoad: vi.fn(),
      localModelUnload: vi.fn(),
      localJobCancel: vi.fn(),
    },
  };
});

beforeAll(() => setLanguage("ja"));

function renderPanel() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <LocalRuntimePanel />
    </QueryClientProvider>,
  );
}

describe("LocalRuntimePanel", () => {
  it("shows runtime state and native model metadata", async () => {
    renderPanel();
    expect(await screen.findByText("LM Studio")).toBeInTheDocument();
    expect(screen.getByText("qwen/qwen3-8b")).toBeInTheDocument();
    expect(screen.getByText("qwen3")).toBeInTheDocument();
    expect(screen.getByText("8B")).toBeInTheDocument();
    expect(screen.getByText("4bit")).toBeInTheDocument();
    expect(screen.getByText("4.3 GB")).toBeInTheDocument();
  });

  it("starts a stopped runtime", async () => {
    const user = userEvent.setup();
    renderPanel();
    const ollama = await screen.findByText("Ollama");
    const card = ollama.closest("div.rounded-xl");
    expect(card).not.toBeNull();
    const start = screen.getAllByRole("button", { name: "起動" })[0];
    await user.click(start);
    await waitFor(() =>
      expect(api.localRuntimeStart).toHaveBeenCalledWith("ollama"),
    );
  });

  it("loads a model through the runtime action", async () => {
    const user = userEvent.setup();
    renderPanel();
    await screen.findByText("qwen/qwen3-8b");
    const load = screen.getByTitle("ロード");
    await user.click(load);
    await waitFor(() =>
      expect(api.localModelLoad).toHaveBeenCalledWith(
        "lmstudio",
        "qwen/qwen3-8b",
      ),
    );
  });

  it("re-detects with a real sweep, not a snapshot refetch", async () => {
    const user = userEvent.setup();
    renderPanel();
    await screen.findByText("LM Studio");
    const snapshotsBefore = vi.mocked(api.localRuntimeSnapshot).mock.calls
      .length;
    await user.click(screen.getByRole("button", { name: "再検出" }));
    await waitFor(() => expect(api.localRuntimeRefresh).toHaveBeenCalled());
    // Re-detect must run a real sweep, not just ask the snapshot for its
    // cached table again.
    expect(vi.mocked(api.localRuntimeSnapshot).mock.calls.length).toBe(
      snapshotsBefore,
    );
  });

  it("fetches logs only when the panel is opened", async () => {
    const user = userEvent.setup();
    renderPanel();
    await screen.findByText("LM Studio");
    expect(api.localRuntimeLogs).not.toHaveBeenCalled();
    await user.click(screen.getAllByRole("button", { name: "ログ" })[0]);
    expect(
      await screen.findByText("listening on 127.0.0.1:1234"),
    ).toBeInTheDocument();
    expect(api.localRuntimeLogs).toHaveBeenCalledWith("lmstudio");
  });
});
