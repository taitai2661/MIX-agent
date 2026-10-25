import { beforeAll, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { setLanguage } from "@/lib/i18n";
import { Composer } from "./Composer";

const models = [
  {
    id: "p:vision",
    label: "Vision Model",
    provider_id: "p",
    context_window: 128000,
    supports_vision: true,
    supports_tool_calling: true,
    supports_reasoning: true,
    auto_candidate: true,
  },
  {
    id: "p:text",
    label: "Text Model",
    provider_id: "p",
    context_window: 128000,
    supports_vision: false,
    supports_tool_calling: true,
    supports_reasoning: true,
    auto_candidate: true,
  },
];

// jsdom reports en-US; pin the language so placeholders are deterministic.
beforeAll(() => setLanguage("ja"));

describe("Composer", () => {
  it("sends on Enter and clears the draft", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        busy={false}
        onSend={onSend}
        onStop={() => {}}
      />,
    );
    const box = screen.getByPlaceholderText("MIX-agent に話しかける…");
    await user.type(box, "こんにちは{Enter}");
    expect(onSend).toHaveBeenCalledWith("こんにちは", [], {
      temporaryMode: false,
      allowTools: false,
    });
    expect((box as HTMLTextAreaElement).value).toBe("");
  });

  it("inserts a newline on Shift+Enter instead of sending", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        busy={false}
        onSend={onSend}
        onStop={() => {}}
      />,
    );
    await user.type(
      screen.getByPlaceholderText("MIX-agent に話しかける…"),
      "a{Shift>}{Enter}{/Shift}b",
    );
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.getByPlaceholderText("MIX-agent に話しかける…")).toHaveValue(
      "a\nb",
    );
  });

  it("does not send while busy and shows stop", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        busy
        onSend={onSend}
        onStop={() => {}}
      />,
    );
    await user.type(
      screen.getByPlaceholderText("MIX-agent に話しかける…"),
      "x{Enter}",
    );
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: /停止/ })).toBeInTheDocument();
  });

  it("disables send when no model is configured", () => {
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={[]}
        modelId=""
        setModelId={() => {}}
        busy={false}
        onSend={() => {}}
        onStop={() => {}}
      />,
    );
    expect(screen.getByRole("button", { name: /送信/ })).toBeDisabled();
  });

  // The selected model is gone because its provider was deleted. Sending would
  // queue a run against a model id nothing can serve, so the composer blocks it
  // and says why instead of letting the run fail downstream.
  it("blocks sending when the selected model is no longer in the list", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={models}
        modelId="p:deleted"
        setModelId={() => {}}
        busy={false}
        onSend={onSend}
        onStop={() => {}}
      />,
    );
    await user.type(
      screen.getByPlaceholderText("MIX-agent に話しかける…"),
      "hello{Enter}",
    );
    expect(onSend).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: /送信/ })).toBeDisabled();
    expect(
      screen.getByText(
        "選択したモデルのプロバイダーが削除されています。モデルを再選択してください。",
      ),
    ).toBeInTheDocument();
  });

  // An empty list is "nothing is configured yet", not "the selection went
  // stale" — the two states need different copy, and the fresh-load state must
  // not claim a provider was deleted.
  it("does not report a stale model while the list is still empty", () => {
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={[]}
        modelId="p:deleted"
        setModelId={() => {}}
        busy={false}
        onSend={() => {}}
        onStop={() => {}}
      />,
    );
    expect(screen.queryByText("選択したモデルが利用できません")).toBeNull();
  });

  // `auto` is resolved server-side, so it is never stale and never blocked —
  // even with an empty model list the send path stays enabled (the backend
  // reports "no Auto candidates" if there really is nothing to serve).
  it("allows sending with Auto even when the model list is empty", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={[]}
        modelId="auto"
        setModelId={() => {}}
        busy={false}
        onSend={onSend}
        onStop={() => {}}
      />,
    );
    await user.type(
      screen.getByPlaceholderText("MIX-agent に話しかける…"),
      "hello{Enter}",
    );
    expect(onSend).toHaveBeenCalledWith("hello", [], {
      temporaryMode: false,
      allowTools: false,
    });
  });

  it("renders a static mode badge when only one mode is allowed (desktop-agent surface)", () => {
    render(
      <Composer
        mode="agent"
        setMode={() => {}}
        allowedModes={["agent"]}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        busy={false}
        onSend={() => {}}
        onStop={() => {}}
      />,
    );
    // The badge text appears but no trigger button is present.
    expect(screen.getByText("エージェント")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "モード" })).toBeNull();
  });

  it("renders the mode dropdown trigger when more than one mode is allowed", () => {
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        allowedModes={["chat", "thinking"]}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        busy={false}
        onSend={() => {}}
        onStop={() => {}}
      />,
    );
    // Trigger button is a plain <button> with aria-label="モード".
    expect(screen.getByRole("button", { name: "モード" })).toBeInTheDocument();
  });

  it("hides the project picker on the chat surface", () => {
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        allowedModes={["chat", "thinking"]}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        projects={[
          {
            id: "p1",
            name: "demo",
            instructions: "",
            rootPath: "/tmp/demo",
            additionalPaths: [],
            sources: [],
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-01T00:00:00Z",
          },
        ]}
        projectId={null}
        onProjectChange={() => {}}
        busy={false}
        onSend={() => {}}
        onStop={() => {}}
      />,
    );
    expect(
      screen.queryByRole("button", { name: "プロジェクトを選択" }),
    ).toBeNull();
  });

  it("shows the project picker on the desktop-agent surface", () => {
    render(
      <Composer
        mode="agent"
        setMode={() => {}}
        allowedModes={["agent"]}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        projects={[
          {
            id: "p1",
            name: "demo",
            instructions: "",
            rootPath: "/tmp/demo",
            additionalPaths: [],
            sources: [],
            created_at: "2026-01-01T00:00:00Z",
            updated_at: "2026-01-01T00:00:00Z",
          },
        ]}
        projectId="p1"
        onProjectChange={() => {}}
        busy={false}
        onSend={() => {}}
        onStop={() => {}}
      />,
    );
    expect(
      screen.getByRole("button", { name: "プロジェクトを選択" }),
    ).toBeInTheDocument();
  });

  it("sends on Cmd+Enter even with Shift held", async () => {
    const onSend = vi.fn();
    const user = userEvent.setup();
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        busy={false}
        onSend={onSend}
        onStop={() => {}}
      />,
    );
    const box = screen.getByPlaceholderText("MIX-agent に話しかける…");
    await user.type(box, "hello");
    fireEvent.keyDown(box, { key: "Enter", shiftKey: true, metaKey: true });
    expect(onSend).toHaveBeenCalledWith("hello", [], {
      temporaryMode: false,
      allowTools: false,
    });
  });

  it("scopes drafts per conversation key", async () => {
    const user = userEvent.setup();
    const { rerender, unmount } = render(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        busy={false}
        onSend={() => {}}
        onStop={() => {}}
        draftKey="thread-a"
      />,
    );
    await user.type(
      screen.getByPlaceholderText("MIX-agent に話しかける…"),
      "draft-a",
    );
    rerender(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        busy={false}
        onSend={() => {}}
        onStop={() => {}}
        draftKey="thread-b"
      />,
    );
    expect(screen.getByPlaceholderText("MIX-agent に話しかける…")).toHaveValue(
      "",
    );
    rerender(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        busy={false}
        onSend={() => {}}
        onStop={() => {}}
        draftKey="thread-a"
      />,
    );
    expect(screen.getByPlaceholderText("MIX-agent に話しかける…")).toHaveValue(
      "draft-a",
    );
    unmount();
  });

  it("does not send when Enter is pressed right after IME composition", async () => {
    const onSend = vi.fn();
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        busy={false}
        onSend={onSend}
        onStop={() => {}}
      />,
    );
    const box = screen.getByPlaceholderText("MIX-agent に話しかける…");
    fireEvent.change(box, { target: { value: "日本語入力" } });
    fireEvent.compositionStart(box);
    fireEvent.compositionEnd(box);
    // Enter pressed immediately on IME confirmation
    fireEvent.keyDown(box, { key: "Enter", keyCode: 13 });
    expect(onSend).not.toHaveBeenCalled();
    expect(box).toHaveValue("日本語入力");
  });

  it("supports modifier-based send shortcut (Cmd+Enter sends, Enter breaks)", async () => {
    const onSend = vi.fn();
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        busy={false}
        onSend={onSend}
        onStop={() => {}}
        sendShortcut="modifier"
      />,
    );
    const box = screen.getByPlaceholderText("MIX-agent に話しかける…");
    fireEvent.change(box, { target: { value: "first line" } });
    // Normal Enter should NOT send in modifier mode
    fireEvent.keyDown(box, { key: "Enter", keyCode: 13 });
    expect(onSend).not.toHaveBeenCalled();

    // Cmd+Enter sends in modifier mode
    fireEvent.keyDown(box, { key: "Enter", keyCode: 13, metaKey: true });
    expect(onSend).toHaveBeenCalledWith("first line", [], {
      temporaryMode: false,
      allowTools: false,
    });
    expect(box).toHaveValue("");
  });

  it("navigates prompt history with ArrowUp and ArrowDown", async () => {
    const onSend = vi.fn();
    render(
      <Composer
        mode="chat"
        setMode={() => {}}
        models={models}
        modelId="p:text"
        setModelId={() => {}}
        busy={false}
        onSend={onSend}
        onStop={() => {}}
      />,
    );
    const box = screen.getByPlaceholderText("MIX-agent に話しかける…");

    // Send first message
    fireEvent.change(box, { target: { value: "first message" } });
    fireEvent.keyDown(box, { key: "Enter", keyCode: 13 });
    expect(box).toHaveValue("");

    // Send second message
    fireEvent.change(box, { target: { value: "second message" } });
    fireEvent.keyDown(box, { key: "Enter", keyCode: 13 });
    expect(box).toHaveValue("");

    // ArrowUp recalls second message
    fireEvent.keyDown(box, { key: "ArrowUp" });
    expect(box).toHaveValue("second message");

    // ArrowUp again recalls first message
    fireEvent.keyDown(box, { key: "ArrowUp" });
    expect(box).toHaveValue("first message");

    // ArrowDown goes back to second message
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(box).toHaveValue("second message");

    // ArrowDown again restores empty draft
    fireEvent.keyDown(box, { key: "ArrowDown" });
    expect(box).toHaveValue("");
  });
});
