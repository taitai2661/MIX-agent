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
});
