import type { Row } from "@/app/api";
import { t } from "@/app/i18n";
import { ArrowDown } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { ChatMessage } from "./ChatMessage";
import { ChatRunPanels } from "./ChatRunPanels";
import type { ApprovalDecision, ConversationRun, Feedback, Run } from "./types";
import type { ToolCallHistory } from "./RunActivity";

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
  onEditChange: (value: string) => void;
  onEditCancel: () => void;
  onEditSave: (message: Row) => void;
  onEditStart: (message: Row) => void;
  onRate: (
    messageId: string,
    current: Feedback,
    value: Exclude<Feedback, null>,
  ) => void;
  onApproval: (
    approvalId: string,
    decision: ApprovalDecision,
  ) => void | Promise<void>;
  onBrowserChange: (browserRunId: string) => void;
  onRetryRun: () => void;
  onError: (error: unknown) => void;
  onResumed: () => void;
  onBudgetResolved: () => void;
};

/** Message list plus its auto-scroll / scroll-to-bottom behaviour. */
export function ChatTranscript({
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
  onEditChange,
  onEditCancel,
  onEditSave,
  onEditStart,
  onRate,
  onApproval,
  onBrowserChange,
  onRetryRun,
  onError,
  onResumed,
  onBudgetResolved,
}: ChatTranscriptProps) {
  const messagesRef = useRef<HTMLDivElement>(null);
  // True while the transcript is pinned to the newest message. Streaming must
  // not yank the viewport away from a user who scrolled up to read history.
  const [atBottom, setAtBottom] = useState(true);
  const atBottomRef = useRef(true);

  useEffect(() => {
    if (!atBottomRef.current) return;
    const el = messagesRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [streamText, messages]);
  useEffect(() => {
    // Entering another conversation starts at its newest message.
    const el = messagesRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
    atBottomRef.current = true;
    setAtBottom(true);
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
        {messages?.map((m) => (
          <ChatMessage
            key={m.id}
            message={m}
            runs={runs}
            allToolCalls={allToolCalls}
            calls={callsByMessage.get(m.id) || []}
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
            onEditChange={onEditChange}
            onEditCancel={onEditCancel}
            onEditSave={onEditSave}
            onEditStart={onEditStart}
            onRate={onRate}
            onApproval={onApproval}
            onBrowserChange={onBrowserChange}
          />
        ))}
        <ChatRunPanels
          streamText={streamText}
          reasoning={reasoning}
          active={active}
          run={run}
          runId={runId}
          fetchedAt={fetchedAt}
          canRetry={canRetry}
          retryDisabled={active || savingEdit}
          onRetry={onRetryRun}
          onError={onError}
          onResumed={onResumed}
          onBudgetResolved={onBudgetResolved}
        />
      </div>
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
}
