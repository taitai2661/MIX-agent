/// One transcript row: persisted user / assistant messages with inline edit,
/// retry and run-error surfaces. Extracted from `Transcript.tsx` (which keeps
/// only the scroll container, the empty-state greeting and the live block)
/// so the list file stays focused.

import { memo, useEffect, useRef, useState, type ReactNode } from "react";
import { FileText, MessageSquare, Pencil, Sparkles } from "lucide-react";
import { Markdown } from "@/components/Markdown";
import { CopyButton } from "@/components/CopyButton";
import { t } from "@/lib/i18n";
import { cn } from "@/lib/cn";
import { displayModelName, relativeTime, stripLeakedMarkup } from "@/lib/format";
import type { MessageView, ModelView, RunView } from "@/lib/types";
import { Reasoning } from "./LiveBlock";
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
      />
    );
  }

  return (
    <div className="group/msg flex gap-3">
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
          <time className="shrink-0 tabular-nums">
            {relativeTime(message.created_at)}
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
          <span className="actions">
            <CopyButton value={message.content} />
          </span>
        </div>
        {message.reasoning && <Reasoning text={message.reasoning} />}
        <div className="text-sm text-fg">
          <Markdown>{stripLeakedMarkup(message.content)}</Markdown>
        </div>
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
}: {
  message: MessageView;
  run: RunView | undefined;
  onEdit?: (messageId: string, content: string) => Promise<void>;
  onRetry?: () => Promise<void> | void;
  editDisabled: boolean;
  retryDisabled?: boolean;
  showRunError: boolean;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(message.content);
  const [saving, setSaving] = useState(false);
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

  const save = async () => {
    if (!canSave || !onEdit) return;
    setSaving(true);
    try {
      await onEdit(message.id, draft.trim());
      setEditing(false);
    } catch {
      // The parent renders the failure; keeping the editor open preserves the
      // typed text instead of dropping it on a transient error.
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="group/msg flex flex-col items-end gap-1">
      {(message.attachments ?? []).length > 0 && (
        <div className="flex flex-wrap justify-end gap-1.5">
          {(message.attachments ?? []).map((a, i) => (
            <span
              key={i}
              className="inline-flex items-center gap-1 rounded-md border border-line bg-inset px-1.5 py-0.5 text-xs text-muted"
            >
              <FileText size={12} />
              {a.filename ?? a.mime_type}
            </span>
          ))}
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
              {hintWithKeys(t("⌘+Enter で保存"), ["⌘+Enter"])}
            </span>
            <Button
              size="sm"
              disabled={saving}
              onClick={() => setEditing(false)}
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
        </div>
      ) : (
        <>
          <div className="max-w-[85%] whitespace-pre-wrap rounded-2xl rounded-br-md border border-brand/25 bg-brand-soft px-3.5 py-2.5 text-sm leading-6 text-fg shadow-xs">
            {message.content}
          </div>
          <div className="flex items-center gap-1 px-1 text-xs text-faint">
            <time className="tabular-nums">
              {relativeTime(message.created_at)}
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
    <Alert tone="danger" className={className}>
      <strong className="block font-medium">{t("実行に失敗しました")}</strong>
      <span className="whitespace-pre-wrap break-words">{error}</span>
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
