/// One transcript row: persisted user / assistant messages with inline edit,
/// retry and run-error surfaces. Extracted from `Transcript.tsx` (which keeps
/// only the scroll container, the empty-state greeting and the live block)
/// so the list file stays focused.

import {
  memo,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useQuery } from "@tanstack/react-query";
import {
  CheckCircle2,
  ChevronRight,
  FileText,
  ListChecks,
  MessageSquare,
  Pencil,
  RefreshCw,
  Sparkles,
  Square,
  Wrench,
} from "lucide-react";
import { api } from "@/lib/api";
import { reduceRunEvents } from "@/lib/runEvents";
import { Markdown } from "@/components/Markdown";
import { CopyButton } from "@/components/CopyButton";
import { t, tf } from "@/lib/i18n";
import { isMacPlatform } from "@/lib/platform";
import { cn } from "@/lib/cn";
import {
  absoluteTime,
  displayModelName,
  relativeTime,
  stripLeakedMarkup,
} from "@/lib/format";
import type { MessageView, ModelView, RunView } from "@/lib/types";
import { Reasoning } from "./LiveBlock";
import { InlineToolItem } from "./InlineToolItem";
import { useUi } from "@/lib/ui";
import { Alert } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Kbd } from "@/components/layout";
export const MessageItem = memo(function MessageItem({
  message,
  runs,
  models = [],
  onEdit,
  onRetry,
  editDisabled = false,
  retryDisabled = false,
  showRunError = false,
  tailCount = 0,
  now,
}: {
  message: MessageView;
  runs: Map<string, RunView>;
  models?: ModelView[];
  /** Edit this user turn and regenerate; absent on read-only transcripts. */
  onEdit?: (messageId: string, content: string) => Promise<void>;
  onRetry?: () => Promise<void> | void;
  editDisabled?: boolean;
  retryDisabled?: boolean;
  /** Render this Run's failure under this (its last assistant) message. */
  showRunError?: boolean;
  /** Messages after this one. Editing this turn deletes all of them, so a
   *  non-zero tail has to be confirmed before the save is sent. */
  tailCount?: number;
  /** Shared minute clock from `Transcript`; keeps relative labels fresh without
   *  one interval per row. */
  now: number;
}) {
  const setActivityRunId = useUi((s) => s.setActivityRunId);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const run = message.run_id ? runs.get(message.run_id) : undefined;

  if (message.role === "user") {
    return (
      <UserMessage
        message={message}
        run={run}
        onEdit={onEdit}
        onRetry={onRetry}
        editDisabled={editDisabled}
        retryDisabled={retryDisabled}
        showRunError={showRunError}
        tailCount={tailCount}
        now={now}
      />
    );
  }

  return (
    <div className="message-row group/msg flex gap-3">
      <span
        aria-hidden="true"
        className="mt-0.5 grid size-6 shrink-0 place-items-center rounded-lg border border-line bg-gradient-to-b from-surface to-inset text-muted shadow-xs"
      >
        <Sparkles size={12} />
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <div
          className={cn(
            "flex items-center gap-1.5 text-xs text-faint",
            // Hover-reveal on precise pointers; always visible on touch
            // (no hover) and small screens so the copy action is reachable.
            "[&_.actions]:ml-auto [&_.actions]:opacity-100",
            "[@media(hover:hover)]:[&_.actions]:opacity-0",
            "[@media(hover:hover)]:group-hover/msg:[&_.actions]:opacity-100 focus-within:[&_.actions]:opacity-100",
          )}
        >
          <span
            className="max-w-48 truncate font-medium text-muted"
            title={message.model_id ?? undefined}
          >
            {displayModelName(message.model_id, models)}
          </span>
          <span aria-hidden="true">·</span>
          <time
            className="shrink-0 tabular-nums"
            dateTime={message.created_at}
            title={absoluteTime(message.created_at)}
          >
            {relativeTime(message.created_at, new Date(now))}
          </time>
          {run && run.tool_call_count > 0 && (
            <Badge asChild tone="info" size="sm">
              <button
                type="button"
                onClick={() => {
                  setActivityRunId(run.id);
                  setRightPanel("activity");
                }}
              >
                <MessageSquare size={10} /> {run.tool_call_count}{" "}
                {t("件のツール")}
              </button>
            </Badge>
          )}
          {run && (run.failed_tool_count ?? 0) > 0 && (
            <Badge asChild tone="danger" size="sm">
              <button
                type="button"
                onClick={() => {
                  setActivityRunId(run.id);
                  setRightPanel("activity");
                }}
              >
                <Wrench size={10} />{" "}
                {tf("{count}件失敗", { count: run.failed_tool_count ?? 0 })}
              </button>
            </Badge>
          )}
          <span className="actions">
            {showRunError &&
              onRetry &&
              run &&
              run.status !== "cancelled" &&
              run.status !== "failed" && (
                <Button
                  variant="ghost"
                  size="icon-sm"
                  disabled={retryDisabled}
                  onClick={() => void onRetry()}
                  title={t("この回答を破棄して再生成")}
                  aria-label={t("回答を再生成")}
                >
                  <RefreshCw size={13} aria-hidden="true" />
                </Button>
              )}
            <CopyButton value={message.content} />
          </span>
        </div>
        {message.reasoning && <Reasoning text={message.reasoning} />}
        <div className="text-sm text-fg">
          <Markdown>{stripLeakedMarkup(message.content)}</Markdown>
        </div>
        {run?.status === "cancelled" && (
          <div
            role="status"
            className="flex flex-wrap items-center gap-1.5 text-xs text-muted"
          >
            <span className="inline-flex items-center gap-1 rounded-full border border-line bg-inset px-2 py-0.5">
              <Square size={9} aria-hidden="true" />
              {t("停止しました")}
            </span>
            {showRunError && onRetry && (
              <Button
                size="sm"
                variant="ghost"
                disabled={retryDisabled}
                onClick={() => void onRetry()}
              >
                {t("再実行")}
              </Button>
            )}
          </div>
        )}
        {run && run.tool_call_count > 0 && (
          <MessageToolsAccordion
            runId={run.id}
            count={run.tool_call_count}
            failedCount={run.failed_tool_count ?? 0}
          />
        )}
        {showRunError && run?.error && (
          <RunErrorAlert
            error={run.error}
            onRetry={onRetry}
            retryDisabled={retryDisabled}
          />
        )}
      </div>
    </div>
  );
});

function MessageToolsAccordion({
  runId,
  count,
  failedCount = 0,
}: {
  runId: string;
  count: number;
  failedCount?: number;
}) {
  // A run with a failed edit/command must not look like a clean success: open
  // the details by default so the failure is on screen without a click.
  const [open, setOpen] = useState(failedCount > 0);
  const events = useQuery({
    queryKey: ["run_events", runId],
    queryFn: () => api.listRunEvents(runId),
    enabled: open,
    staleTime: Infinity,
  });

  const timeline = useMemo(
    () => (events.data ? reduceRunEvents(runId, events.data) : null),
    [events.data, runId],
  );

  const todos = timeline?.todos ?? [];
  const completedCount = todos.filter((t) => t.status === "completed").length;

  return (
    <details
      open={open}
      onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}
      className="group mt-1 rounded-lg border border-line bg-surface/80 text-xs shadow-xs"
    >
      <summary className="flex cursor-pointer list-none items-center justify-between px-2.5 py-1.5 transition-colors hover:bg-hover">
        <div className="flex items-center gap-1.5 text-muted">
          <Wrench
            size={12}
            className="shrink-0 text-faint"
            aria-hidden="true"
          />
          <span className="font-medium text-fg">{t("実行されたツール")}</span>
          <Badge tone="neutral" size="sm">
            {count}
          </Badge>
          {failedCount > 0 && (
            <Badge tone="danger" size="sm">
              {tf("{count}件失敗", { count: failedCount })}
            </Badge>
          )}
          {todos.length > 0 && (
            <Badge
              tone={completedCount === todos.length ? "success" : "brand"}
              size="sm"
            >
              {completedCount}/{todos.length}
            </Badge>
          )}
        </div>
        <ChevronRight
          size={12}
          className="shrink-0 text-faint transition-transform group-open:rotate-90"
          aria-hidden="true"
        />
      </summary>
      <div className="flex flex-col gap-2 border-t border-line p-2">
        {events.isLoading && (
          <div className="flex items-center gap-1.5 py-2 text-faint">
            <Spinner size={12} />
            <span>{t("読み込み中…")}</span>
          </div>
        )}
        {todos.length > 0 && (
          <div className="flex flex-col gap-1 rounded-md border border-line/60 bg-inset/50 p-2">
            <div className="flex items-center gap-1.5 font-medium text-fg">
              <ListChecks size={13} className="text-brand" />
              <span>{t("計画")}</span>
              <span className="text-faint text-2xs">
                ({completedCount}/{todos.length})
              </span>
            </div>
            <ul className="m-0 flex flex-col gap-1 p-0 list-none">
              {todos.map((item) => (
                <li
                  key={item.id}
                  className="flex items-start gap-1.5 text-2xs leading-4"
                >
                  <span className="mt-0.5 shrink-0">
                    {item.status === "completed" ? (
                      <CheckCircle2 size={11} className="text-success" />
                    ) : item.status === "in_progress" ? (
                      <Spinner size={10} className="text-warning" />
                    ) : (
                      <span className="block size-2.5 rounded-sm border border-line" />
                    )}
                  </span>
                  <span
                    className={
                      item.status === "completed"
                        ? "text-faint line-through"
                        : "text-fg"
                    }
                  >
                    {item.content}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        )}
        {timeline?.tools.map((tool) => (
          <InlineToolItem key={tool.id} tool={tool} defaultOpen={false} />
        ))}
      </div>
    </details>
  );
}

/** A user turn with its inline editor. Kept separate from `MessageItem` so the
 *  editing state does not re-render the (memoised) assistant rows, and so the
 *  textarea's hooks only run for messages that actually carry them. */
function UserMessage({
  message,
  run,
  onEdit,
  onRetry,
  editDisabled,
  retryDisabled = false,
  showRunError,
  tailCount = 0,
  now,
}: {
  message: MessageView;
  run: RunView | undefined;
  onEdit?: (messageId: string, content: string) => Promise<void>;
  onRetry?: () => Promise<void> | void;
  editDisabled: boolean;
  retryDisabled?: boolean;
  showRunError: boolean;
  tailCount?: number;
  now: number;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(message.content);
  const [saving, setSaving] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  // Re-seed from the persisted text whenever the editor opens, and focus it so
  // the user can type without reaching for the mouse.
  useEffect(() => {
    if (!editing) return;
    setDraft(message.content);
    const frame = window.requestAnimationFrame(() => {
      const el = textareaRef.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [editing, message.content]);

  const canSave =
    draft.trim().length > 0 &&
    draft.trim() !== message.content.trim() &&
    !saving;
  const saveHint = isMacPlatform() ? "⌘+Enter" : "Ctrl+Enter";
  const saveHintText = isMacPlatform()
    ? t("⌘+Enter で保存")
    : t("Ctrl+Enter で保存");

  const save = async () => {
    if (!canSave || !onEdit) return;
    // The backend truncates the conversation from this turn onward. That is
    // the intent of an edit, but it is destructive and irreversible, and the
    // editor previously looked like an ordinary save: one click on an old
    // prompt silently erased every later message, tool call and file change.
    // Confirm while a tail exists, and name what will be removed.
    if (tailCount > 0 && !confirming) {
      setConfirming(true);
      return;
    }
    setSaving(true);
    try {
      await onEdit(message.id, draft.trim());
      setEditing(false);
      setConfirming(false);
    } catch {
      // The parent renders the failure; keeping the editor open preserves the
      // typed text instead of dropping it on a transient error.
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="message-row group/msg flex flex-col items-end gap-1">
      {(message.attachments ?? []).length > 0 && (
        <div className="flex max-w-full min-w-0 flex-wrap justify-end gap-1.5">
          {(message.attachments ?? []).map((a, i) => {
            const attachmentName = a.filename ?? a.mime_type;
            return (
              <span
                key={i}
                className="inline-flex max-w-full min-w-0 items-center gap-1 rounded-md border border-line bg-inset px-1.5 py-0.5 text-xs text-muted"
              >
                <FileText size={12} className="shrink-0" />
                <span className="truncate" title={attachmentName}>
                  {attachmentName}
                </span>
              </span>
            );
          })}
        </div>
      )}
      {editing ? (
        <div className="flex w-full max-w-[85%] flex-col items-end gap-1.5">
          <textarea
            ref={textareaRef}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                setConfirming(false);
                setEditing(false);
                return;
              }
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                void save();
              }
            }}
            rows={Math.min(8, draft.split("\n").length)}
            aria-label={t("メッセージを編集")}
            className="max-h-56 w-full resize-none rounded-2xl rounded-br-md border border-brand/40 bg-surface px-3.5 py-2.5 text-sm leading-6 text-fg outline-none focus:border-brand focus:ring-2 focus:ring-brand/15"
          />
          <div className="flex items-center gap-1.5 px-1">
            <span className="text-xs text-faint">
              {hintWithKeys(saveHintText, [saveHint])}
            </span>
            <Button
              size="sm"
              disabled={saving}
              onClick={() => {
                setConfirming(false);
                setEditing(false);
              }}
            >
              {t("キャンセル")}
            </Button>
            <Button
              variant="primary"
              size="sm"
              disabled={!canSave}
              onClick={() => void save()}
            >
              {saving ? <Spinner size={12} /> : t("保存")}
            </Button>
          </div>
          {confirming && (
            <div
              role="alert"
              className="flex w-full flex-col gap-2 rounded-xl border border-warning/40 bg-warning-soft px-3 py-2 text-xs text-fg"
            >
              <p className="m-0">
                {tf(
                  "この後の{count}件のメッセージとツール実行履歴を削除して、この発言から再生成します。元に戻せません。",
                  { count: tailCount },
                )}
              </p>
              <div className="flex items-center justify-end gap-1.5">
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => setConfirming(false)}
                >
                  {t("キャンセル")}
                </Button>
                <Button
                  size="sm"
                  variant="danger"
                  disabled={saving}
                  onClick={() => void save()}
                >
                  {saving ? <Spinner size={12} /> : t("削除して保存")}
                </Button>
              </div>
            </div>
          )}
        </div>
      ) : (
        <>
          <div className="min-w-0 max-w-[85%] rounded-2xl rounded-br-md border border-brand/25 bg-brand-soft px-3.5 py-2.5 text-sm leading-6 whitespace-pre-wrap break-words text-fg shadow-xs">
            {message.content}
          </div>
          <div className="flex items-center gap-1 px-1 text-xs text-faint">
            <time
              className="tabular-nums"
              dateTime={message.created_at}
              title={absoluteTime(message.created_at)}
            >
              {relativeTime(message.created_at, new Date(now))}
            </time>
            {onEdit && (
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={() => setEditing(true)}
                disabled={editDisabled}
                aria-label={t("メッセージを編集")}
                title={
                  editDisabled
                    ? t("実行中は編集できません")
                    : t("メッセージを編集")
                }
                className="text-faint hover:text-fg [@media(hover:hover)]:opacity-0 [@media(hover:hover)]:group-hover/msg:opacity-100 [@media(hover:hover)]:focus-visible:opacity-100"
              >
                <Pencil size={13} aria-hidden="true" />
              </Button>
            )}
          </div>
        </>
      )}
      {showRunError && run?.error && (
        <RunErrorAlert
          error={run.error}
          onRetry={onRetry}
          retryDisabled={retryDisabled}
          className="w-full"
        />
      )}
    </div>
  );
}

function RunErrorAlert({
  error,
  onRetry,
  retryDisabled = false,
  className,
}: {
  error: string;
  onRetry?: () => Promise<void> | void;
  retryDisabled?: boolean;
  className?: string;
}) {
  const [retrying, setRetrying] = useState(false);
  const retry = async () => {
    if (!onRetry || retryDisabled || retrying) return;
    setRetrying(true);
    try {
      await onRetry();
    } finally {
      setRetrying(false);
    }
  };
  return (
    <Alert
      tone="danger"
      className={className}
      action={<CopyButton value={error} label={t("エラー内容をコピー")} />}
    >
      <div className="flex flex-col gap-1">
        <strong className="block font-medium">{t("実行に失敗しました")}</strong>
        <span className="whitespace-pre-wrap break-words text-xs leading-5">
          {error}
        </span>
        {onRetry && (
          <span className="mt-2 flex items-center gap-2">
            <Button
              variant="default"
              size="sm"
              disabled={retryDisabled || retrying}
              onClick={() => void retry()}
            >
              {retrying ? <Spinner size={12} /> : null}
              {t("再試行")}
            </Button>
            {retryDisabled && (
              <span className="text-xs text-faint">
                {t("実行中は再試行できません")}
              </span>
            )}
          </span>
        )}
      </div>
    </Alert>
  );
}

/** Render shortcut hints inside a translated string as `<Kbd>` chips.
 *
 *  Extracted alongside `MessageItem`: both the empty-state greeting and the
 *  edit-save hint bold their keys this way, so the hint stays one i18n string
 *  instead of being split into per-language fragments. */
export function hintWithKeys(text: string, keys: readonly string[]): ReactNode {
  const pattern = new RegExp(
    `(${keys.map((k) => k.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`,
    "g",
  );
  return text
    .split(pattern)
    .map((part, i) => (keys.includes(part) ? <Kbd key={i}>{part}</Kbd> : part));
}
