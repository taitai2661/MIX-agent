import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { setLanguage } from "@/lib/i18n";
import { api } from "@/lib/api";
import { DEFAULT_SETTINGS } from "@/lib/settings";
import type { SettingsView } from "@/lib/types";
import { Onboarding } from "./Onboarding";

vi.mock("@/lib/api", () => ({
  api: {
    listProviders: vi.fn(async () => []),
    listModels: vi.fn(async () => []),
    providerPresets: vi.fn(async () => []),
    listProjects: vi.fn(async () => []),
    listConversations: vi.fn(async () => []),
    getSettings: vi.fn(async () => ({ ...DEFAULT_SETTINGS })),
    updateSettings: vi.fn(async (input: SettingsView) => input),
    createProvider: vi.fn(async () => ({
      id: "pv1",
      kind: "openai",
      label: "OpenAI",
      base_url: "https://api.openai.com/v1",
      enabled: true,
      has_api_key: true,
      preset: "openai",
    })),
    updateProvider: vi.fn(async () => {
      throw new Error("not under test");
    }),
    connectProvider: vi.fn(async () => 1),
    createProject: vi.fn(async () => ({
      id: "p1",
      name: "demo",
      instructions: "",
      rootPath: "/tmp/demo",
      additionalPaths: [],
      sources: [],
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
    })),
    createConversation: vi.fn(async () => ({
      id: "c1",
      title: "新しいスレッド",
      mode: "chat",
      model_id: "m1",
      updated_at: "2026-01-01T00:00:00Z",
      preview: null,
      message_count: 0,
      project_id: null,
    })),
    // The local tab mounts `LocalRuntimeScan` even though the sweep is only
    // reachable behind a click, so these have to exist for the module to render.
    scanLocalRuntimes: vi.fn(async () => ({
      runtimes: [],
      occupied_ports: [],
      hosts: ["127.0.0.1"],
      elapsed_ms: 1,
      any_online: false,
    })),
    probeLocalEndpoint: vi.fn(async () => {
      throw new Error("not under test");
    }),
    importLocalRuntime: vi.fn(async () => {
      throw new Error("not under test");
    }),
  },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => "/tmp/demo"),
}));

const mockedApi = vi.mocked(api);

// jsdom reports en-US; pin the language so labels are deterministic.
beforeAll(() => setLanguage("ja"));

beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
  mockedApi.listProviders.mockResolvedValue([]);
  mockedApi.listModels.mockResolvedValue([]);
  mockedApi.providerPresets.mockResolvedValue([]);
  mockedApi.listProjects.mockResolvedValue([]);
  mockedApi.getSettings.mockResolvedValue({ ...DEFAULT_SETTINGS });
  mockedApi.updateSettings.mockImplementation(async (input) => input);
  mockedApi.connectProvider.mockResolvedValue(1);
  mockedApi.createProvider.mockResolvedValue({
    id: "pv1",
    kind: "openai",
    label: "OpenAI",
    base_url: "https://api.openai.com/v1",
    enabled: true,
    has_api_key: true,
    preset: "openai",
  });
});

const PRESET = {
  slug: "openai",
  label: "OpenAI",
  category: "直接接続",
  kind: "openai",
  default_url: "https://api.openai.com/v1",
  api_key_required: true,
  allow_private_default: false,
  notes: "",
};

function renderWizard(
  props: Partial<React.ComponentProps<typeof Onboarding>> = {},
) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const onComplete = props.onComplete ?? vi.fn();
  const onDefer = props.onDefer ?? vi.fn();
  const view = render(
    <QueryClientProvider client={client}>
      <Onboarding onComplete={onComplete} onDefer={onDefer} />
    </QueryClientProvider>,
  );
  return { ...view, onComplete, onDefer };
}

/** Welcome → step 1 (model provider). */
async function reachProviderStep(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "始める" }));
  await screen.findByTestId("onboarding-provider-tab-local");
}

/** Step 1 → step 2, by actually connecting a cloud provider. */
async function reachFolderStep(user: ReturnType<typeof userEvent.setup>) {
  mockedApi.providerPresets.mockResolvedValue([PRESET]);
  await reachProviderStep(user);
  await user.click(screen.getByTestId("onboarding-provider-tab-cloud"));
  await user.selectOptions(screen.getByRole("combobox"), "openai");
  await user.type(screen.getByPlaceholderText("sk-…"), "sk-test");
  await user.click(
    screen.getByRole("button", { name: "接続してモデル一覧を取得" }),
  );
  await screen.findByRole("button", { name: "フォルダを選択" });
}

describe("Onboarding", () => {
  it("opens on the welcome page, and numbers the steps only after it", async () => {
    const user = userEvent.setup();
    renderWizard();

    expect(
      screen.getByRole("heading", { name: "ようこそ" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByTestId("onboarding-step-provider"),
    ).not.toBeInTheDocument();

    await reachProviderStep(user);
    expect(screen.getByTestId("onboarding-step-provider")).toBeInTheDocument();
  });

  it("leads with the local route and keeps the cloud option beside it", async () => {
    const user = userEvent.setup();
    renderWizard();
    await reachProviderStep(user);

    // The two provider routes are a radiogroup, not a tablist: they switch which
    // form is shown rather than revealing a sibling panel, and `aria-checked` is
    // what says which one is current. There is deliberately no `tabpanel` for
    // them to point at.
    expect(screen.getByTestId("onboarding-provider-tab-local")).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByTestId("onboarding-provider-tab-cloud")).toHaveAttribute(
      "aria-checked",
      "false",
    );

    await user.click(screen.getByTestId("onboarding-provider-tab-cloud"));
    expect(screen.getByTestId("onboarding-provider-tab-cloud")).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  it("does not let the rail skip past a step that has not produced a model", async () => {
    const user = userEvent.setup();
    renderWizard();
    await reachProviderStep(user);

    expect(screen.getByTestId("onboarding-step-folder")).toBeDisabled();
    expect(screen.getByTestId("onboarding-step-start")).toBeDisabled();
  });

  it("needs a preset before it will connect, then advances on success", async () => {
    const user = userEvent.setup();
    mockedApi.providerPresets.mockResolvedValue([PRESET]);
    renderWizard();
    await reachProviderStep(user);
    await user.click(screen.getByTestId("onboarding-provider-tab-cloud"));

    const connect = screen.getByRole("button", {
      name: "接続してモデル一覧を取得",
    });
    expect(connect).toBeDisabled();

    await user.selectOptions(screen.getByRole("combobox"), "openai");
    // Picking a preset fills the base URL, so it must not have to be typed.
    expect(
      screen.getByDisplayValue("https://api.openai.com/v1"),
    ).toBeInTheDocument();

    // The preset declares that it needs a key, so connecting without one would
    // come back as an opaque HTTP 401.
    expect(connect).toBeDisabled();
    expect(
      screen.getByText("このプロバイダーでは API キーが必要です"),
    ).toBeInTheDocument();

    await user.type(screen.getByPlaceholderText("sk-…"), "sk-test");
    expect(connect).toBeEnabled();
    await user.click(connect);

    await waitFor(() => expect(mockedApi.createProvider).toHaveBeenCalled());
    expect(mockedApi.connectProvider).toHaveBeenCalledWith("pv1");
    // Step 2 is the folder picker.
    await screen.findByRole("button", { name: "フォルダを選択" });
  });

  it("surfaces a rejected credential and stays on the step", async () => {
    const user = userEvent.setup();
    mockedApi.providerPresets.mockResolvedValue([PRESET]);
    mockedApi.connectProvider.mockRejectedValue("model discovery returned 401");
    renderWizard();
    await reachProviderStep(user);
    await user.click(screen.getByTestId("onboarding-provider-tab-cloud"));
    await user.selectOptions(screen.getByRole("combobox"), "openai");
    await user.type(screen.getByPlaceholderText("sk-…"), "sk-wrong");
    await user.click(
      screen.getByRole("button", { name: "接続してモデル一覧を取得" }),
    );

    await screen.findByText("model discovery returned 401");
    expect(
      screen.queryByRole("button", { name: "フォルダを選択" }),
    ).not.toBeInTheDocument();
  });

  it("creates the project from the picked folder and makes it active", async () => {
    const user = userEvent.setup();
    renderWizard();
    await reachFolderStep(user);

    await user.click(screen.getByRole("button", { name: "フォルダを選択" }));
    await waitFor(() =>
      expect(screen.getByText("/tmp/demo")).toBeInTheDocument(),
    );

    const use = screen.getByRole("button", { name: "このフォルダを使う" });
    expect(use).toBeEnabled();
    await user.type(screen.getByPlaceholderText("Project 名"), "demo");
    await user.click(use);

    await waitFor(() =>
      expect(mockedApi.createProject).toHaveBeenCalledWith({
        name: "demo",
        instructions: "",
        rootPath: "/tmp/demo",
        sources: [],
      }),
    );
    // Step 3 summarises what was set up.
    await screen.findByRole("heading", { name: "準備ができました" });
    expect(screen.getByText("demo")).toBeInTheDocument();
  });

  it("lets step 2 be deferred without losing the model that was just added", async () => {
    const user = userEvent.setup();
    renderWizard();
    await reachFolderStep(user);

    await user.click(screen.getByRole("button", { name: "あとから設定する" }));

    await screen.findByRole("heading", { name: "準備ができました" });
    expect(mockedApi.createProject).not.toHaveBeenCalled();
    expect(
      screen.getByText(/未設定（チャットはプロジェクトなしで始まります）/),
    ).toBeInTheDocument();
  });

  it("opens the first thread and finishes", async () => {
    const user = userEvent.setup();
    mockedApi.listProjects.mockResolvedValue([
      {
        id: "p1",
        name: "既存プロジェクト",
        instructions: "",
        rootPath: "/tmp/demo",
        additionalPaths: [],
        sources: [],
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
      },
    ]);
    const { onComplete } = renderWizard();
    await reachFolderStep(user);
    await user.click(
      screen.getByRole("button", { name: "このプロジェクトを使う" }),
    );
    await screen.findByRole("heading", { name: "準備ができました" });

    await user.click(screen.getByRole("button", { name: "チャットを始める" }));

    await waitFor(() => expect(onComplete).toHaveBeenCalled());
    await waitFor(() =>
      expect(mockedApi.createConversation).toHaveBeenCalled(),
    );
  });

  it("hands the skip button to the caller rather than writing settings itself", async () => {
    const user = userEvent.setup();
    const { onDefer } = renderWizard();

    await user.click(
      screen.getAllByRole("button", { name: "あとで設定する" })[0],
    );

    expect(onDefer).toHaveBeenCalled();
    // `App` owns the gate's `complete`, which is what persists the flag.
    expect(mockedApi.updateSettings).not.toHaveBeenCalled();
  });

  it("only sweeps the loopback ports when the scan is asked for", async () => {
    const user = userEvent.setup();
    renderWizard();
    await reachProviderStep(user);

    expect(mockedApi.scanLocalRuntimes).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "検出" }));
    await waitFor(() => expect(mockedApi.scanLocalRuntimes).toHaveBeenCalled());
    // An empty result has to point at the other tab rather than dead-ending.
    expect(
      screen.getByText(/「クラウド API」タブを使ってください/),
    ).toBeInTheDocument();
  });
});
