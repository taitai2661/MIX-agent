import { beforeAll, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { setLanguage } from "@/lib/i18n";
import { Composer } from "./Composer";

const models = [
  { id: "p:vision", label: "Vision Model", provider_id: "p", context_window: 128000, supports_vision: true, supports_tool_calling: true, supports_reasoning: true },
  { id: "p:text", label: "Text Model", provider_id: "p", context_window: 128000, supports_vision: false, supports_tool_calling: true, supports_reasoning: true },
];

// jsdom reports en-US; pin the language so placeholders are deterministic.
beforeAll(() => setLanguage("ja"));

describe("Composer", () => {
  it("sends on Enter and clears the draft", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    render(
      <Composer
        mode="chat" setMode={() => {}} models={models} modelId="p:text" setModelId={() => {}}
        busy={false} onSend={onSend} onStop={() => {}}
      />,
    );
    const box = screen.getByPlaceholderText("MIX-agent に話しかける…");
    await user.type(box, "こんにちは{Enter}");
    expect(onSend).toHaveBeenCalledWith("こんにちは", []);
    expect((box as HTMLTextAreaElement).value).toBe("");
  });

  it("inserts a newline on Shift+Enter instead of sending", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    render(
      <Composer
        mode="chat" setMode={() => {}} models={models} modelId="p:text" setModelId={() => {}}
        busy={false} onSend={onSend} onStop={() => {}}
      />,
    );
    await user.type(screen.getByPlaceholderText("MIX-agent に話しかける…"), "a{Shift>}{Enter}{/Shift}b");
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.getByPlaceholderText("MIX-agent に話しかける…")).toHaveValue("a\nb");
  });

  it("does not send while busy and shows stop", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    render(
      <Composer
        mode="chat" setMode={() => {}} models={models} modelId="p:text" setModelId={() => {}}
        busy onSend={onSend} onStop={() => {}}
      />,
    );
    await user.type(screen.getByPlaceholderText("MIX-agent に話しかける…"), "x{Enter}");
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: /停止/ })).toBeInTheDocument();
  });

  it("disables send when no model is configured", () => {
    render(
      <Composer
        mode="chat" setMode={() => {}} models={[]} modelId="" setModelId={() => {}}
        busy={false} onSend={() => {}} onStop={() => {}}
      />,
    );
    expect(screen.getByRole("button", { name: /送信/ })).toBeDisabled();
  });

  it("renders a static mode badge when only one mode is allowed (desktop-agent surface)", () => {
    render(
      <Composer
        mode="agent" setMode={() => {}} allowedModes={["agent"]}
        models={models} modelId="p:text" setModelId={() => {}}
        busy={false} onSend={() => {}} onStop={() => {}}
      />,
    );
    // The badge text appears but no trigger button is present.
    expect(screen.getByText("エージェント")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "モード" })).toBeNull();
  });

  it("renders the mode dropdown trigger when more than one mode is allowed", () => {
    render(
      <Composer
        mode="chat" setMode={() => {}} allowedModes={["chat", "thinking"]}
        models={models} modelId="p:text" setModelId={() => {}}
        busy={false} onSend={() => {}} onStop={() => {}}
      />,
    );
    // Trigger button is a plain <button> with aria-label="モード".
    expect(screen.getByRole("button", { name: "モード" })).toBeInTheDocument();
  });

  it("hides the project picker on the chat surface", () => {
    render(
      <Composer
        mode="chat" setMode={() => {}} allowedModes={["chat", "thinking"]}
        models={models} modelId="p:text" setModelId={() => {}}
        projects={[{ id: "p1", name: "demo", instructions: "", rootPath: "/tmp/demo", additionalPaths: [], sources: [], created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z" }]}
        projectId={null}
        onProjectChange={() => {}}
        busy={false} onSend={() => {}} onStop={() => {}}
      />,
    );
    expect(screen.queryByRole("button", { name: "プロジェクトを選択" })).toBeNull();
  });

  it("shows the project picker on the desktop-agent surface", () => {
    render(
      <Composer
        mode="agent" setMode={() => {}} allowedModes={["agent"]}
        models={models} modelId="p:text" setModelId={() => {}}
        projects={[{ id: "p1", name: "demo", instructions: "", rootPath: "/tmp/demo", additionalPaths: [], sources: [], created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z" }]}
        projectId="p1"
        onProjectChange={() => {}}
        busy={false} onSend={() => {}} onStop={() => {}}
      />,
    );
    expect(screen.getByRole("button", { name: "プロジェクトを選択" })).toBeInTheDocument();
  });
});
