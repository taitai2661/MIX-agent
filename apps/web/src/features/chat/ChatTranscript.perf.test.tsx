// @vitest-environment jsdom
/// Rendering-cost regressions: memoized messages must survive stream frames,
/// and long conversations must not mount every row at once.
///
/// Assertions are made against the rendered DOM rather than a mocked
/// `ChatMessage` render counter: the cost regression is observable as message
/// rows being re-created or re-written, independent of which component happens
/// to produce them.
import { act, useMemo, useRef, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { getLanguage, setLanguage, type Language } from "@/app/i18n";
import type { Row } from "@/app/api";
import { ChatActionsProvider, type ChatActions } from "./ChatActionsContext";
import { ChatTranscript, type ChatTranscriptProps } from "./ChatTranscript";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

let container: HTMLDivElement;
let root: Root;
let previousLanguage: Language;
let observer: MutationObserver | null = null;
let messageMutations = 0;

const message = (id: string): Row =>
  ({
    id,
    created_at: "2026-01-01T00:00:00Z",
    data: { role: "user", content: id },
  }) as Row;

const baseProps = (
  overrides: Partial<ChatTranscriptProps> = {},
): ChatTranscriptProps => {
  const noop = () => {};
  return {
    messages: [],
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
    actions: {
      onEditChange: noop,
      onEditCancel: noop,
      onEditSave: noop,
      onEditStart: noop,
      onRate: noop,
      onApproval: noop,
      onBrowserChange: noop,
      onRetryRun: noop,
      onError: noop,
      onResumed: noop,
      onBudgetResolved: noop,
    },
    ...overrides,
  };
};

function render(ui: ReactElement) {
  act(() => root.render(ui));
}

function messageRows(): HTMLElement[] {
  // All fixture rows are user turns. Excluding `article.message.assistant`
  // avoids counting the streamed-answer panel that ChatRunPanels renders.
  return Array.from(container.querySelectorAll("article.message.user"));
}

/** Watch only the message rows: stream panels live outside them, so a streamed
 *  frame is allowed to change the DOM as long as history is left untouched. */
function watchMessages() {
  observer?.disconnect();
  messageMutations = 0;
  observer = new MutationObserver((records) => {
    messageMutations += records.length;
  });
  for (const row of messageRows()) {
    observer.observe(row, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true,
    });
  }
}

beforeEach(() => {
  previousLanguage = getLanguage();
  setLanguage("ja");
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  observer = null;
  messageMutations = 0;
});

afterEach(() => {
  observer?.disconnect();
  act(() => root.unmount());
  container.remove();
  setLanguage(previousLanguage);
});

describe("ChatTranscript rendering cost", () => {
  it("does not re-render memoized messages when only stream text changes", () => {
    const messages = [message("m1"), message("m2")];
    const props = baseProps({ messages });
    render(<ChatTranscript {...props} />);
    expect(messageRows()).toHaveLength(2);

    watchMessages();
    render(<ChatTranscript {...props} streamText="ストリーム中" />);
    // No message row was re-written; only the stream panel changed.
    expect(messageMutations).toBe(0);
    expect(container.textContent).toContain("ストリーム中");
  });

  it("mounts only the newest 40 rows in a long conversation", () => {
    const messages = Array.from({ length: 50 }, (_, i) => message(`m${i}`));
    render(<ChatTranscript {...baseProps({ messages })} />);
    expect(messageRows()).toHaveLength(40);
    expect(messageRows().some((row) => row.textContent?.includes("m0"))).toBe(
      false,
    );

    const older = container.querySelector(
      ".show-older-messages",
    ) as HTMLElement;
    expect(older).not.toBeNull();
    expect(older.textContent).toContain("以前のメッセージを表示");
    act(() => older.click());
    expect(messageRows()).toHaveLength(50);
    expect(messageRows().some((row) => row.textContent?.includes("m0"))).toBe(
      true,
    );
    // Revealing history disables auto-follow so the next streamed token cannot
    // yank the viewport back to the newest message.
    expect(container.querySelector(".scroll-to-bottom")).not.toBeNull();
  });

  it("survives a caller that rebuilds every handler implementation on each render", () => {
    const messages = [message("m1"), message("m2")];
    // Keep every non-stream prop referentially stable, as Chat does.
    const stableProps = baseProps({ messages, actions: undefined });
    const freshActions = (): ChatActions => ({
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
    });

    // Mirrors Chat.tsx: implementations are recreated every render, but the
    // object handed to context has a stable identity and delegates through a
    // ref, so memoized message rows can still bail out on streamed frames.
    function Harness({ streamText }: { streamText: string }) {
      const impl = useRef<ChatActions>(null!);
      impl.current = freshActions();
      const stable = useMemo<ChatActions>(
        () => ({
          onEditChange: (value) => impl.current.onEditChange(value),
          onEditCancel: () => impl.current.onEditCancel(),
          onEditSave: (msg) => impl.current.onEditSave(msg),
          onEditStart: (msg) => impl.current.onEditStart(msg),
          onRate: (id, current, value) =>
            impl.current.onRate(id, current, value),
          onApproval: (id, decision, ack) =>
            impl.current.onApproval(id, decision, ack),
          onBrowserChange: (id) => impl.current.onBrowserChange(id),
          onRetryRun: () => impl.current.onRetryRun(),
          onError: (error) => impl.current.onError(error),
          onResumed: () => impl.current.onResumed(),
          onBudgetResolved: () => impl.current.onBudgetResolved(),
        }),
        [],
      );
      return (
        <ChatActionsProvider actions={stable}>
          <ChatTranscript {...stableProps} streamText={streamText} />
        </ChatActionsProvider>
      );
    }

    render(<Harness streamText="a" />);
    expect(messageRows()).toHaveLength(2);
    watchMessages();
    render(<Harness streamText="b" />);
    expect(messageMutations).toBe(0);
    expect(container.textContent).toContain("b");
  });
});
