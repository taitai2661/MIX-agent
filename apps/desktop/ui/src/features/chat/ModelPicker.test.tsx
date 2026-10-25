import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { setLanguage } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { getFavoriteModels, getRecentModels } from "@/lib/modelPrefs";
import type { ModelView, ProviderView } from "@/lib/types";
import { ModelPicker } from "./ModelPicker";

const providers: ProviderView[] = [
  {
    id: "openai",
    kind: "openai",
    label: "OpenAI",
    base_url: null,
    enabled: true,
    has_api_key: true,
    preset: null,
  },
  {
    id: "anthropic",
    kind: "anthropic",
    label: "Anthropic",
    base_url: null,
    enabled: true,
    has_api_key: true,
    preset: null,
  },
];

const models: ModelView[] = [
  {
    id: "openai:gpt-4o",
    provider_id: "openai",
    label: "GPT-4o",
    context_window: 128000,
    supports_vision: true,
    supports_tool_calling: true,
    supports_reasoning: false,
    auto_candidate: true,
  },
  {
    id: "openai:o3",
    provider_id: "openai",
    label: "o3",
    context_window: 200000,
    supports_vision: true,
    supports_tool_calling: true,
    supports_reasoning: true,
    auto_candidate: true,
  },
  {
    id: "anthropic:claude",
    provider_id: "anthropic",
    label: "Claude Sonnet",
    context_window: 200000,
    supports_vision: true,
    supports_tool_calling: true,
    supports_reasoning: true,
    auto_candidate: false,
  },
];

beforeAll(() => setLanguage("ja"));

describe("ModelPicker", () => {
  beforeEach(() => {
    localStorage.clear();
    useUi.setState({ view: "chat", settingsTab: "appearance" });
  });
  afterEach(() => {
    localStorage.clear();
    useUi.setState({ view: "chat", settingsTab: "appearance" });
  });

  it("shows the selected model in the trigger", () => {
    render(
      <ModelPicker
        models={models}
        value="openai:gpt-4o"
        onChange={() => {}}
        providers={providers}
      />,
    );
    expect(
      screen.getByRole("button", { name: "モデルを選択" }),
    ).toHaveTextContent("GPT-4o");
  });

  it("shows Auto in the trigger and lists it first", async () => {
    const user = userEvent.setup();
    render(
      <ModelPicker
        models={models}
        value="auto"
        onChange={() => {}}
        providers={providers}
      />,
    );
    expect(
      screen.getByRole("button", { name: "モデルを選択" }),
    ).toHaveTextContent("Auto");
    await user.click(screen.getByRole("button", { name: "モデルを選択" }));
    const list = await screen.findByRole("listbox");
    const options = within(list).getAllByRole("option");
    expect(options[0]).toHaveTextContent("Auto");
  });

  it("selects Auto on click", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(
      <ModelPicker
        models={models}
        value="openai:gpt-4o"
        onChange={onChange}
        providers={providers}
      />,
    );
    await user.click(screen.getByRole("button", { name: "モデルを選択" }));
    const list = await screen.findByRole("listbox");
    await user.click(within(list).getAllByRole("option")[0]);
    expect(onChange).toHaveBeenCalledWith("auto");
  });

  it("filters the list with the search field", async () => {
    const user = userEvent.setup();
    render(
      <ModelPicker
        models={models}
        value="openai:gpt-4o"
        onChange={() => {}}
        providers={providers}
      />,
    );
    await user.click(screen.getByRole("button", { name: "モデルを選択" }));
    await user.type(screen.getByRole("combobox"), "claude");
    const list = await screen.findByRole("listbox");
    expect(within(list).getByText("Claude Sonnet")).toBeInTheDocument();
    expect(within(list).queryByText("GPT-4o")).toBeNull();
  });

  it("selects a model on click and records it as recent", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(
      <ModelPicker
        models={models}
        value="openai:gpt-4o"
        onChange={onChange}
        providers={providers}
      />,
    );
    await user.click(screen.getByRole("button", { name: "モデルを選択" }));
    await user.click(await screen.findByText("Claude Sonnet"));
    expect(onChange).toHaveBeenCalledWith("anthropic:claude");
    expect(getRecentModels()[0]).toBe("anthropic:claude");
  });

  it("selects with the keyboard", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(
      <ModelPicker
        models={models}
        value="openai:gpt-4o"
        onChange={onChange}
        providers={providers}
      />,
    );
    await user.click(screen.getByRole("button", { name: "モデルを選択" }));
    const combobox = screen.getByRole("combobox");
    await user.type(combobox, "{ArrowDown}{ArrowDown}{Enter}");
    // The list starts on Auto; two downs reach the second model (o3).
    expect(onChange).toHaveBeenCalledWith("openai:o3");
  });

  it("jumps with Home/End and chooses with native option buttons", async () => {
    const onChange = vi.fn();
    const user = userEvent.setup();
    render(
      <ModelPicker
        models={models}
        value="openai:gpt-4o"
        onChange={onChange}
        providers={providers}
      />,
    );
    await user.click(screen.getByRole("button", { name: "モデルを選択" }));
    const combobox = screen.getByRole("combobox");
    // Home lands on Auto, End on the last model; Enter activates natively.
    await user.type(combobox, "{End}");
    const list = await screen.findByRole("listbox");
    const options = within(list).getAllByRole("option");
    expect(options[options.length - 1]).toHaveTextContent("Claude Sonnet");
    await user.type(combobox, "{Home}{Enter}");
    expect(onChange).toHaveBeenCalledWith("auto");
  });

  it("toggles favourites from the keyboard with aria-pressed state", async () => {
    const user = userEvent.setup();
    render(
      <ModelPicker
        models={models}
        value="openai:gpt-4o"
        onChange={() => {}}
        providers={providers}
      />,
    );
    await user.click(screen.getByRole("button", { name: "モデルを選択" }));
    const star = (
      await screen.findAllByRole("button", { name: "お気に入りに追加" })
    )[0];
    expect(star).toHaveAttribute("aria-pressed", "false");
    star.focus();
    await user.keyboard("{Enter}");
    expect(getFavoriteModels()).toContain("openai:gpt-4o");
    expect(
      await screen.findByRole("button", { name: "お気に入りから外す" }),
    ).toHaveAttribute("aria-pressed", "true");
  });

  it("pins a model to favourites", async () => {
    const user = userEvent.setup();
    render(
      <ModelPicker
        models={models}
        value="openai:gpt-4o"
        onChange={() => {}}
        providers={providers}
      />,
    );
    await user.click(screen.getByRole("button", { name: "モデルを選択" }));
    const star = (
      await screen.findAllByRole("button", { name: "お気に入りに追加" })
    )[1];
    await user.click(star);
    expect(getFavoriteModels()).toContain("openai:o3");
    expect(await screen.findByText("お気に入り")).toBeInTheDocument();
  });

  it("offers to add a provider when nothing is configured", async () => {
    const user = userEvent.setup();
    render(
      <ModelPicker models={[]} value="" onChange={() => {}} providers={[]} />,
    );
    await user.click(screen.getByRole("button", { name: "モデルを選択" }));
    await user.click(
      await screen.findByRole("button", { name: "プロバイダーを追加" }),
    );
    await waitFor(() => expect(useUi.getState().settingsTab).toBe("providers"));
  });

  it("toggles temporary mode and tool access", async () => {
    const onTemporaryChange = vi.fn();
    const onAllowToolsChange = vi.fn();
    const user = userEvent.setup();
    render(
      <ModelPicker
        models={models}
        value="openai:gpt-4o"
        onChange={() => {}}
        providers={providers}
        temporary={false}
        onTemporaryChange={onTemporaryChange}
        allowTools={false}
        onAllowToolsChange={onAllowToolsChange}
      />,
    );
    await user.click(screen.getByRole("button", { name: "モデルを選択" }));
    await user.click(screen.getByRole("switch", { name: "一時モード" }));
    expect(onTemporaryChange).toHaveBeenCalledWith(true);
  });
});
