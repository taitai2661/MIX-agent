// @vitest-environment jsdom
import type { Row } from "@/app/api";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ChatComposer, type ChatComposerProps } from "./ChatComposer";
import { ChatMessage, type ChatMessageProps } from "./ChatMessage";
import { ChatTranscript, type ChatTranscriptProps } from "./ChatTranscript";
import { ChatWelcome } from "./ChatWelcome";
import type { Welcome } from "./greeting";
import type { Artifact } from "./types";

(
  globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

const mounted: Array<{ root: Root; container: HTMLDivElement }> = [];

function mount(element: React.ReactNode) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(element));
  mounted.push({ root, container });
  return container;
}

afterEach(() => {
  for (const { root, container } of mounted.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

function click(element: Element) {
  act(() => {
    element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function type(element: HTMLTextAreaElement, value: string) {
  act(() => {
    element.setRangeText(value, 0, element.value.length, "end");
    element.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function keydown(element: Element, init: KeyboardEventInit) {
  act(() => {
    element.dispatchEvent(
      new KeyboardEvent("keydown", { bubbles: true, ...init }),
    );
  });
}

const message = (data: Record<string, unknown>): Row =>
  ({ id: "m1", created_at: "2026-01-01T00:00:00Z", data }) as Row;

const messageProps = (
  overrides: Partial<ChatMessageProps> = {},
): ChatMessageProps => ({
  message: message({ role: "user", content: "こんにちは" }),
  calls: [],
  liveCalls: [],
  active: false,
  runId: "",
  editing: false,
  editText: "",
  savingEdit: false,
  editDisabled: false,
  showTodos: false,
  onEditChange: () => {},
  onEditCancel: () => {},
  onEditSave: () => {},
  onEditStart: () => {},
  onRate: () => {},
  onApproval: () => {},
  onBrowserChange: () => {},
  ...overrides,
});

const composerProps = (
  overrides: Partial<ChatComposerProps> = {},
): ChatComposerProps => ({
  text: "こんにちは",
  onTextChange: () => {},
  onSubmit: () => {},
  textareaRef: { current: null },
  attachments: [],
  onRemoveAttachment: () => {},
  onAttach: () => {},
  uploading: false,
  models: [],
  agents: [],
  projects: [],
  tools: [],
  permissionRules: [],
  model: "auto",
  onModelChange: () => {},
  mode: "chat",
  onModeChange: () => {},
  researchMode: false,
  onResearchModeChange: () => {},
  agent: "",
  onAgentChange: () => {},
  selectedProject: "",
  onSelectedProjectChange: () => {},
  temporaryMode: false,
  onTemporaryModeChange: () => {},
  allowTools: false,
  onAllowToolsChange: () => {},
  active: false,
  busy: false,
  needsProviderSetup: false,
  hasNoProviders: false,
  pageError: null,
  onRetry: () => {},
  onConnectProvider: () => {},
  onStop: () => {},
  ...overrides,
});

const transcriptProps = (
  overrides: Partial<ChatTranscriptProps> = {},
): ChatTranscriptProps => ({
  messages: [message({ role: "user", content: "こんにちは" })],
  callsByMessage: new Map(),
  liveCalls: [],
  active: false,
  runId: "",
  streamText: "",
  reasoning: "",
  fetchedAt: 0,
  editingId: null,
  editText: "",
  savingEdit: false,
  editDisabled: false,
  canRetry: false,
  onEditChange: () => {},
  onEditCancel: () => {},
  onEditSave: () => {},
  onEditStart: () => {},
  onRate: () => {},
  onApproval: () => {},
  onBrowserChange: () => {},
  onRetryRun: () => {},
  onError: () => {},
  onResumed: () => {},
  onBudgetResolved: () => {},
  ...overrides,
});

const welcome: Welcome = {
  id: "welcome",
  title: ["今日は"],
  titleIds: ["title.1"],
  note: "安全に作業できます",
  noteId: "note.1",
  suggestions: [{ id: "s1", label: "調べる", prompt: "これを調べて" }],
  used: [],
};

describe("chat component interactions", () => {
  it("wires composer text, shortcut, mode, toggle, attachment, and stop actions", () => {
    const onTextChange = vi.fn();
    const onSubmit = vi.fn();
    const onModeChange = vi.fn();
    const onResearchModeChange = vi.fn();
    const onAttach = vi.fn();
    const onRemoveAttachment = vi.fn();
    const onStop = vi.fn();
    const attachments: Artifact[] = [
      { artifact_id: "a1", name: "notes.txt", mime: "text/plain", size: 10 },
    ];
    const container = mount(
      <MemoryRouter>
        <ChatComposer
          {...composerProps({
            text: "",
            onTextChange,
            onSubmit,
            onModeChange,
            onResearchModeChange,
            onAttach,
            onRemoveAttachment,
            onStop,
            attachments,
          })}
        />
      </MemoryRouter>,
    );

    const textarea = container.querySelector<HTMLTextAreaElement>("textarea")!;
    type(textarea, "新しいメッセージ");
    expect(onTextChange).toHaveBeenLastCalledWith("新しいメッセージ");

    const paste = new Event("paste", { bubbles: true }) as Event & {
      clipboardData: { items: Array<{ kind: string; getAsFile: () => File }> };
    };
    const file = new File(["hello"], "note.txt", { type: "text/plain" });
    Object.defineProperty(paste, "clipboardData", {
      value: { items: [{ kind: "file", getAsFile: () => file }] },
    });
    void act(() => textarea.dispatchEvent(paste));
    expect(onAttach).toHaveBeenCalledWith([file]);

    keydown(textarea, { key: "Enter" });
    expect(onSubmit).toHaveBeenCalled();

    click(container.querySelectorAll(".mode-switch button")[2]);
    expect(onModeChange).toHaveBeenCalledWith("agent");

    click(container.querySelector('[role="switch"]')!);
    expect(onResearchModeChange).toHaveBeenCalledWith(true);

    click(container.querySelector(".attachments button")!);
    expect(onRemoveAttachment).toHaveBeenCalledWith(attachments[0]);

    const fileInput =
      container.querySelector<HTMLInputElement>('input[type="file"]')!;
    Object.defineProperty(fileInput, "files", {
      configurable: true,
      value: [file],
    });
    void act(() =>
      fileInput.dispatchEvent(new Event("change", { bubbles: true })),
    );
    expect(onAttach).toHaveBeenCalledWith([file]);

    click(container.querySelector(".composer-actions button.icon")!);

    const activeContainer = mount(
      <MemoryRouter>
        <ChatComposer {...composerProps({ active: true, onStop })} />
      </MemoryRouter>,
    );
    click(activeContainer.querySelector(".composer-actions .send")!);
    expect(onStop).toHaveBeenCalled();
  });

  it("wires message editing, feedback, and save shortcuts", () => {
    const onEditStart = vi.fn();
    const onEditChange = vi.fn();
    const onEditCancel = vi.fn();
    const onEditSave = vi.fn();
    const onRate = vi.fn();
    const user = messageProps({ onEditStart });
    const container = mount(<ChatMessage {...user} />);
    click(container.querySelector(".message-edit-trigger")!);
    expect(onEditStart).toHaveBeenCalledWith(user.message);

    const editing = messageProps({
      message: message({ role: "user", content: "old" }),
      editing: true,
      editText: "new",
      onEditChange,
      onEditCancel,
      onEditSave,
    });
    const editingContainer = mount(<ChatMessage {...editing} />);
    const textarea = editingContainer.querySelector("textarea")!;
    type(textarea, "changed");
    expect(onEditChange).toHaveBeenLastCalledWith("changed");
    keydown(textarea, { key: "Escape" });
    expect(onEditCancel).toHaveBeenCalled();
    keydown(textarea, { key: "Enter", ctrlKey: true });
    expect(onEditSave).toHaveBeenCalledWith(editing.message);
    click(editingContainer.querySelectorAll(".message-edit-actions button")[1]);
    expect(onEditSave).toHaveBeenCalledTimes(2);

    const assistant = messageProps({
      message: message({
        role: "assistant",
        content: "回答",
        auto_selection: { model_name: "Test Model" },
        feedback: "up",
      }),
      onRate,
    });
    const assistantContainer = mount(<ChatMessage {...assistant} />);
    const feedbackButtons = assistantContainer.querySelectorAll(
      ".message-auto button",
    );
    expect(feedbackButtons).toHaveLength(2);
    click(feedbackButtons[0]);
    expect(onRate).toHaveBeenLastCalledWith("m1", "up", "up");
    click(feedbackButtons[1]);
    expect(onRate).toHaveBeenLastCalledWith("m1", "up", "down");
  });

  it("scrolls the transcript and reveals the jump-to-latest control", () => {
    Object.defineProperty(HTMLElement.prototype, "scrollTo", {
      configurable: true,
      value: () => {},
    });
    window.matchMedia = (() => ({
      matches: false,
      addEventListener: () => {},
      removeEventListener: () => {},
    })) as unknown as typeof window.matchMedia;
    const container = mount(<ChatTranscript {...transcriptProps()} />);
    const messages = container.querySelector<HTMLElement>(".messages")!;
    Object.defineProperty(messages, "scrollHeight", {
      configurable: true,
      value: 1000,
    });
    Object.defineProperty(messages, "clientHeight", {
      configurable: true,
      value: 100,
    });
    Object.defineProperty(messages, "scrollTop", {
      configurable: true,
      value: 0,
      writable: true,
    });
    void act(() =>
      messages.dispatchEvent(new Event("scroll", { bubbles: true })),
    );
    const jump = container.querySelector(".scroll-to-bottom")!;
    expect(jump).not.toBeNull();
    click(jump);
    expect(container.querySelector(".scroll-to-bottom")).toBeNull();
  });

  it("passes a clicked suggestion to the composer", () => {
    const onPickSuggestion = vi.fn();
    const container = mount(
      <ChatWelcome
        loading={false}
        welcome={welcome}
        onPickSuggestion={onPickSuggestion}
      />,
    );
    click(container.querySelector(".suggestions button")!);
    expect(onPickSuggestion).toHaveBeenCalledWith("これを調べて");
  });
});
