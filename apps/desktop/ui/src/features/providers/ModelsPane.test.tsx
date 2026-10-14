import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { setLanguage } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { api } from "@/lib/api";
import { ModelsPane } from "./ModelsPane";

vi.mock("@/lib/api", () => ({
  api: {
    listModels: vi.fn(async () => [
      { id: "openai:gpt-4o", provider_id: "openai", label: "GPT-4o", context_window: 128000, supports_vision: true, supports_tool_calling: true, supports_reasoning: false, auto_candidate: true },
      { id: "anthropic:claude", provider_id: "anthropic", label: "Claude Sonnet", context_window: 200000, supports_vision: true, supports_tool_calling: true, supports_reasoning: true, auto_candidate: false },
    ]),
    listProviders: vi.fn(async () => [
      { id: "openai", kind: "openai", label: "OpenAI", base_url: null, enabled: true, has_api_key: true, preset: null },
      { id: "anthropic", kind: "anthropic", label: "Anthropic", base_url: null, enabled: true, has_api_key: true, preset: null },
    ]),
    verifyTools: vi.fn(async () => ({})),
    setModelAuto: vi.fn(async () => ({})),
  },
}));

const mockedApi = vi.mocked(api);

beforeAll(() => setLanguage("ja"));

function renderPane() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ModelsPane />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  useUi.setState({ settingsTab: "appearance" });
});

describe("ModelsPane", () => {
  it("groups models by provider", async () => {
    renderPane();
    expect(await screen.findByText("GPT-4o")).toBeInTheDocument();
    expect(screen.getByText("Claude Sonnet")).toBeInTheDocument();
    expect(screen.getAllByText("OpenAI").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Anthropic").length).toBeGreaterThan(0);
  });

  it("filters by the search box", async () => {
    const user = userEvent.setup();
    renderPane();
    await screen.findByText("GPT-4o");
    await user.type(screen.getByRole("textbox", { name: "モデルを検索" }), "claude");
    expect(screen.queryByText("GPT-4o")).toBeNull();
    expect(screen.getByText("Claude Sonnet")).toBeInTheDocument();
  });

  it("sends the user to providers when nothing is configured", async () => {
    mockedApi.listModels.mockResolvedValueOnce([]);
    const user = userEvent.setup();
    renderPane();
    await user.click(await screen.findByRole("button", { name: "プロバイダーを追加" }));
    await waitFor(() => expect(useUi.getState().settingsTab).toBe("providers"));
  });

  it("toggles Auto candidacy per model", async () => {
    const user = userEvent.setup();
    renderPane();
    await screen.findByText("GPT-4o");
    const switches = await screen.findAllByRole("switch", { name: "Auto候補にする" });
    expect(switches).toHaveLength(2);
    await user.click(switches[1]);
    await waitFor(() =>
      expect(mockedApi.setModelAuto).toHaveBeenCalledWith("anthropic:claude", true),
    );
  });
});
