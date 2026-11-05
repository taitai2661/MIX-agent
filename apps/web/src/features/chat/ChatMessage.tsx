import type { Row } from "@/app/api";
import { t } from "@/app/i18n";
import { isMacPlatform } from "@/app/platform";
import { Button } from "@/components/button";
import { Pencil, ThumbsDown, ThumbsUp } from "lucide-react";
import { memo } from "react";

import { ArtifactCard, type ChatArtifact } from "./ArtifactCard";
import { BrowserPanel } from "./BrowserPanel";
import { ChatMarkdown } from "./ChatMarkdown";
import { RunActivity, type ToolCallHistory } from "./RunActivity";
import { TodoPane } from "./TodoPane";
import type { ApprovalDecision, ConversationRun, Feedback, Run } from "./types";

export type ChatMessageProps = {
  message: Row;
  /** Stored runs for the conversation, used for summaries and browser panels. */
  runs?: ConversationRun[];
  /** Persisted tool-call history for the whole conversation. */
  allToolCalls?: ToolCallHistory[];
  /** Persisted calls that belong to this message. */
  calls: ToolCallHistory[];
  /** In-flight calls derived from the SSE stream. */
  liveCalls: ToolCallHistory[];
  currentRunMessageId?: string | null;
  active: boolean;
  runId: string;
  run?: Run;
  editing: boolean;
  editText: string;
  savingEdit: boolean;
  editDisabled: boolean;
  showTodos: boolean;
  todos?: unknown;
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
};

/** One transcript row: context summary, message body, tools, todos, browser. */
function ChatMessageImpl({
  message,
  runs,
  allToolCalls,
  calls,
  liveCalls,
  currentRunMessageId,
  active,
  runId,
  run,
  editing,
  editText,
  savingEdit,
  editDisabled,
  showTodos,
  todos,
  onEditChange,
  onEditCancel,
  onEditSave,
  onEditStart,
  onRate,
  onApproval,
  onBrowserChange,
}: ChatMessageProps) {
  const m = message;
  const role = typeof m.data.role === "string" ? m.data.role : "";
  const isUser = role === "user";
  const rawContent = m.data.content;
  const content =
    typeof rawContent === "string"
      ? rawContent
      : rawContent == null
        ? ""
        : String(rawContent);
  const runningHere = active && currentRunMessageId === m.id;
  const mergedCalls = (() => {
    if (!runningHere) return calls;
    const known = new Set(calls.map((call) => call.id));
    return [...calls, ...liveCalls.filter((call) => !known.has(call.id))];
  })();
  const contextSummaries =
    runs?.filter((r) => r.message_id === m.id && r.context_summary?.text) || [];
  const browserRuns =
    runs?.filter(
      (r) =>
        r.message_id === m.id &&
        allToolCalls?.some(
          (call) =>
            call.run_id === r.id && call.tool_name.startsWith("browser_"),
        ),
    ) || [];

  return (
    <div>
      {isUser &&
        contextSummaries.map((r) => (
          <details className="context-summary" key={r.id}>
            <summary>
              {t("この実行で文脈を圧縮しました ·")}{" "}
              {r.context_summary?.covered_count ?? 0}
              {t("件を要約")}
            </summary>
            <p>
              {t(
                "古い会話は以下の要約としてモデルに渡しました。元の会話は画面に残っています。",
              )}
            </p>
            <div className="context-summary-text">
              {r.context_summary?.text}
            </div>
          </details>
        ))}
      <article className={"message " + role}>
        <div className="message-label">
          {isUser ? (
            t("あなた")
          ) : (
            <>
              <span className="mini-mark">M</span>MIX agent
            </>
          )}
        </div>
        {editing ? (
          <div className="message-edit">
            {/* Focus follows the user's explicit "編集" action. */}
            {/* eslint-disable jsx-a11y/no-autofocus -- user-initiated edit mode */}
            <textarea
              aria-label={t("メッセージを編集")}
              value={editText}
              autoFocus
              onChange={(e) => onEditChange(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  onEditCancel();
                }
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  onEditSave(m);
                }
              }}
            />
            {/* eslint-enable jsx-a11y/no-autofocus */}
            <div className="message-edit-actions">
              <span className="message-edit-hint">
                {isMacPlatform() ? t("⌘+Enter で保存") : t("Ctrl+Enter で保存")}
              </span>
              <Button
                type="button"
                variant="ghost"
                onClick={onEditCancel}
                disabled={savingEdit}
              >
                {t("キャンセル")}
              </Button>
              <Button
                type="button"
                onClick={() => onEditSave(m)}
                disabled={savingEdit || !editText.trim()}
              >
                {t("保存")}
              </Button>
            </div>
          </div>
        ) : (
          <div className="markdown">
            <ChatMarkdown>{content}</ChatMarkdown>
          </div>
        )}
        {isUser && !editing && (
          <button
            type="button"
            className="message-edit-trigger"
            aria-label={t("メッセージを編集")}
            title={t("メッセージを編集")}
            disabled={editDisabled}
            onClick={() => onEditStart(m)}
          >
            <Pencil size={12} /> {t("編集")}
          </button>
        )}
        {Array.isArray(m.data.artifacts) && m.data.artifacts.length
          ? (m.data.artifacts as ChatArtifact[])
              .filter(
                (artifact: ChatArtifact): artifact is ChatArtifact =>
                  typeof artifact === "object" &&
                  artifact !== null &&
                  typeof (artifact as ChatArtifact).artifact_id === "string",
              )
              .map((artifact: ChatArtifact) => (
                <ArtifactCard key={artifact.artifact_id} artifact={artifact} />
              ))
          : Array.isArray(m.data.artifact_ids)
            ? (m.data.artifact_ids as unknown[])
                .filter(
                  (artifact_id): artifact_id is string =>
                    typeof artifact_id === "string" && artifact_id.length > 0,
                )
                .map((artifact_id: string) => (
                  <ArtifactCard key={artifact_id} artifact={{ artifact_id }} />
                ))
            : null}
        {(() => {
          const performance = m.data.performance as
            | {
                output_tokens?: unknown;
                generation_ms?: unknown;
                tokens_per_second?: unknown;
              }
            | undefined;
          const tps = performance?.tokens_per_second;
          if (typeof tps !== "number" || !Number.isFinite(tps)) return null;
          const output =
            typeof performance?.output_tokens === "number" &&
            Number.isFinite(performance.output_tokens)
              ? performance.output_tokens
              : "?";
          const ms =
            typeof performance?.generation_ms === "number" &&
            Number.isFinite(performance.generation_ms)
              ? performance.generation_ms
              : "?";
          return (
            <div
              className="message-performance"
              title={t("出力 {tokens} tokens / 生成 {ms}ms", {
                tokens: String(output),
                ms: String(ms),
              })}
            >
              {tps.toFixed(1)} tokens/s
            </div>
          );
        })()}
        {m.data.auto_selection && (
          <div className="message-auto">
            <small>
              Auto:{" "}
              {(() => {
                const selection = m.data.auto_selection as {
                  model_name?: unknown;
                  model_id?: unknown;
                };
                const name =
                  typeof selection.model_name === "string" &&
                  selection.model_name
                    ? selection.model_name
                    : typeof selection.model_id === "string"
                      ? selection.model_id
                      : selection.model_id == null
                        ? ""
                        : String(selection.model_id);
                return name;
              })()}
            </small>
            <span>
              <button
                className={m.data.feedback === "up" ? "selected" : ""}
                onClick={() => onRate(m.id, m.data.feedback, "up")}
                aria-label={t("良い回答")}
              >
                <ThumbsUp size={14} />
              </button>
              <button
                className={m.data.feedback === "down" ? "selected" : ""}
                onClick={() => onRate(m.id, m.data.feedback, "down")}
                aria-label={t("良くない回答")}
              >
                <ThumbsDown size={14} />
              </button>
            </span>
          </div>
        )}
      </article>
      {isUser && (
        <RunActivity
          running={runningHere}
          approvals={run?.approvals}
          calls={mergedCalls}
          onApproval={onApproval}
        />
      )}
      {isUser && showTodos && <TodoPane items={todos} />}
      {isUser &&
        browserRuns.map((r) => (
          <BrowserPanel
            key={r.id}
            runId={r.id}
            run={r.id === runId ? run : undefined}
            onChange={() => onBrowserChange(r.id)}
          />
        ))}
    </div>
  );
}

/**
 * React.memo matters here: while a run streams, `streamText` changes several
 * times a second and re-renders the transcript. Without this, every message
 * (and its Markdown tree) re-parsed on each token, including all history.
 */
export const ChatMessage = memo(ChatMessageImpl);
