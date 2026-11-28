import type { Row } from "@/app/api";
import { t } from "@/app/i18n";
import { ArrowDown } from "lucide-react";
import {
  memo,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import {
  ChatActionsProvider,
  FALLBACK_ACTIONS,
  useChatActionsContext,
  type ChatActions,
} from "./ChatActionsContext";
import { ChatMessage } from "./ChatMessage";
import { ChatRunPanels } from "./ChatRunPanels";
import type { ConversationRun, Run } from "./types";
import type { ToolCallHistory } from "./RunActivity";

/**
 * `@tanstack/react-virtual` is not a dependency of the web app, so long
 * conversations use a lightweight floor: only the newest rows are mounted and
 * older ones are revealed in batches. Same cutoff as the desktop transcript.
 */
const VIRTUALIZE_AFTER = 40;

/** Stable empty array — a fresh `[]` per row defeats memo(ChatMessage). */
const EMPTY_CALLS: ToolCallHistory[] = [];

export type ChatTranscriptProps = {
  conversationId?: string;
  messages?: Row[];
  runs?: ConversationRun[];
  todos?: unknown;
  allToolCalls?: ToolCallHistory[];
  callsByMessage: Map<string, ToolCallHistory[]>;
  liveCalls: ToolCallHistory[];
  currentRunMessageId?: string | null;
  latestUserMessageId?: string;
  active: boolean;
  runId: string;
  run?: Run;
  streamText: string;
  reasoning: string;
  fetchedAt: number;
  editingId: string | null;
  editText: string;
  savingEdit: boolean;
  editDisabled: boolean;
  canRetry: boolean;
  /** Bundled callbacks from Chat; falls back to the surrounding context. */
  actions?: ChatActions;
};

/** Message list plus its auto-scroll / scroll-to-bottom behaviour. */
export const ChatTranscript = memo(function ChatTranscript({
  conversationId,
  messages,
  runs,
  todos,
  allToolCalls,
  callsByMessage,
  liveCalls,
  currentRunMessageId,
  latestUserMessageId,
  active,
  runId,
  run,
  streamText,
  reasoning,
  fetchedAt,
  editingId,
  editText,
  savingEdit,
  editDisabled,
  canRetry,
  actions,
}: ChatTranscriptProps) {
  const contextActions = useChatActionsContext();
  const a = actions ?? contextActions ?? FALLBACK_ACTIONS;
  const messagesRef = useRef<HTMLDivElement>(null);
  // True while the transcript is pinned to the newest message. Streaming must
  // not yank the viewport away from a user who scrolled up to read history.
  const [atBottom, setAtBottom] = useState(true);
  const atBottomRef = useRef(true);
  const [visibleCount, setVisibleCount] = useState(VIRTUALIZE_AFTER);
  // Revealing older rows prepends DOM, which shifts the viewport. Remember the
  // pre-insertion geometry and restore the same visual position in a layout
  // effect (before paint) instead of letting the user jump to the oldest row.
  const pendingScrollAdjust = useRef<{ height: number; top: number } | null>(
    null,
  );
  const totalMessages = messages?.length ?? 0;
  const virtualized = totalMessages > VIRTUALIZE_AFTER;
  const firstVisible = virtualized
    ? Math.max(0, totalMessages - visibleCount)
    : 0;
  const visibleMessages = virtualized
    ? messages!.slice(firstVisible)
    : (messages ?? []);
  // Announce the streamed answer at most once every two seconds. Flushing the
  // last 500 characters on every animation frame made screen readers repeat
  // half-sentences continuously and drowned out navigation.
  const [announcement, setAnnouncement] = useState("");
  const latestStream = useRef(streamText);
  latestStream.current = streamText;
  const lastAssistantMessageId = useMemo(
    () =>
      [...(messages ?? [])]
        .reverse()
        .find((message) => message.data.role === "assistant")?.id,
    [messages],
  );
  useEffect(() => {
    if (!active) {
      setAnnouncement("");
      return;
    }
    const timer = window.setInterval(
      () => setAnnouncement(latestStream.current.slice(-400)),
      2000,
    );
    return () => window.clearInterval(timer);
  }, [active]);

  useEffect(() => {
    if (!atBottomRef.current) return;
    const el = messagesRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [streamText, messages]);
  useLayoutEffect(() => {
    const pending = pendingScrollAdjust.current;
    if (!pending) return;
    pendingScrollAdjust.current = null;
    const el = messagesRef.current;
    if (!el) return;
    el.scrollTop = pending.top + (el.scrollHeight - pending.height);
  }, [visibleCount]);
  useEffect(() => {
    // Entering another conversation starts at its newest message.
    const el = messagesRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
    atBottomRef.current = true;
    setAtBottom(true);
    setVisibleCount(VIRTUALIZE_AFTER);
  }, [conversationId]);
  const handleMessagesScroll = () => {
    const el = messagesRef.current;
    if (!el) return;
    const next = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    if (next !== atBottomRef.current) {
      atBottomRef.current = next;
      setAtBottom(next);
    }
  };
  const scrollToBottom = () => {
    const el = messagesRef.current;
    if (!el) return;
    el.scrollTo({
      top: el.scrollHeight,
      behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
        ? "auto"
        : "smooth",
    });
    atBottomRef.current = true;
    setAtBottom(true);
  };

  return (
    <div className="messages-wrap">
      <div
        className="messages"
        ref={messagesRef}
        onScroll={handleMessagesScroll}
      >
        {virtualized && firstVisible > 0 && (
          <button
            type="button"
            className="show-older-messages"
            onClick={() => {
              const el = messagesRef.current;
              if (el)
                pendingScrollAdjust.current = {
                  height: el.scrollHeight,
                  top: el.scrollTop,
                };
              // Reading history is an explicit opt-out of auto-follow: the next
              // streamed token must not yank the viewport back to the bottom.
              atBottomRef.current = false;
              setAtBottom(false);
              setVisibleCount((count) => count + VIRTUALIZE_AFTER);
            }}
          >
            {t("以前のメッセージを表示")} ({firstVisible} {t("件")})
          </button>
        )}
        <ChatActionsProvider actions={a}>
          {visibleMessages.map((m) => (
            <ChatMessage
              key={m.id}
              message={m}
              runs={runs}
              allToolCalls={allToolCalls}
              calls={callsByMessage.get(m.id) || EMPTY_CALLS}
              liveCalls={liveCalls}
              currentRunMessageId={currentRunMessageId}
              active={active}
              runId={runId}
              run={run}
              editing={editingId === m.id}
              editText={editText}
              savingEdit={savingEdit}
              editDisabled={editDisabled}
              showTodos={
                m.data.role === "user" &&
                (currentRunMessageId
                  ? currentRunMessageId === m.id
                  : latestUserMessageId === m.id)
              }
              todos={todos}
              isLastAssistant={lastAssistantMessageId === m.id}
              onRegenerate={canRetry ? a.onRetryRun : undefined}
            />
          ))}
        </ChatActionsProvider>
        <ChatRunPanels
          streamText={streamText}
          reasoning={reasoning}
          active={active}
          run={run}
          runId={runId}
          fetchedAt={fetchedAt}
          canRetry={canRetry}
          retryDisabled={active || savingEdit}
          onRetry={a.onRetryRun}
          onError={a.onError}
          onResumed={a.onResumed}
          onBudgetResolved={a.onBudgetResolved}
        />
      </div>
      {/* Streaming text is otherwise silent for screen readers: the transcript
          re-renders visually but nothing is announced. A polite region keeps it
          from interrupting navigation, and the tail bounds how much is read. */}
      <span
        className="sr-only"
        role="status"
        aria-live="polite"
        aria-atomic="false"
      >
        {active ? announcement : ""}
      </span>
      {!atBottom && (
        <button
          type="button"
          className="scroll-to-bottom"
          onClick={scrollToBottom}
          aria-label={t("最新のメッセージへ")}
          title={t("最新のメッセージへ")}
        >
          <ArrowDown size={15} />
          <span>{t("最新へ")}</span>
        </button>
      )}
    </div>
  );
});
