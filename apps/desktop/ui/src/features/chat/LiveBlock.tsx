/// Live run block: the streaming answer plus its tool activity, approval and
/// budget cards. Extracted from `Transcript.tsx` so the transcript file owns
/// only persisted history.
///
/// Accessibility: terminal states (error) render with `role="status"` so
/// assistive tech announces the outcome without the transcript container
/// needing an aggressive live region.

import { memo, useEffect, useState } from "react";
import {
  Brain,
  CheckCircle2,
  ChevronRight,
  ListChecks,
  ShieldAlert,
  Sparkles,
} from "lucide-react";
import { InlineToolItem } from "./InlineToolItem";
import { Markdown } from "@/components/Markdown";
import { stripLeakedMarkup } from "@/lib/format";
import { t, tf } from "@/lib/i18n";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { api } from "@/lib/api";
import {
  isBusy,
  type RunTimeline,
  type TodoItem,
  type ToolActivity,
} from "@/lib/runEvents";
import { Alert } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { CopyButton } from "@/components/CopyButton";
import { ErrorBadge } from "@/components/shared";
import { ApprovalCard } from "./ApprovalCard";

export const Reasoning = memo(function Reasoning({
  text,
  streaming = false,
}: {
  text: string;
  streaming?: boolean;
}) {
  const [open, setOpen] = useState(streaming);
  useEffect(() => {
    if (streaming) setOpen(true);
  }, [streaming]);

  return (
    <details
      open={open}
      onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}
      className="group rounded-lg border border-line bg-inset/60 shadow-xs"
    >
      <summary className="flex cursor-pointer list-none items-center justify-between rounded-lg px-3 py-2 text-xs font-medium text-muted transition-colors hover:bg-hover hover:text-fg">
        <div className="flex min-w-0 items-center gap-1.5">
          <ChevronRight
            size={12}
            aria-hidden="true"
            className="shrink-0 transition-transform duration-150 group-open:rotate-90"
          />
          <Brain
            size={12}
            className={cn("shrink-0", streaming && "text-brand animate-pulse")}
            aria-hidden="true"
          />
          <span>{t("思考プロセス")}</span>
          {streaming ? (
            <span className="flex items-center gap-1 text-2xs text-brand font-normal">
              <Spinner size={9} />
              <span>{t("思考中…")}</span>
            </span>
          ) : (
            <span className="text-2xs text-faint font-normal">
              ({text.length} {t("文字")})
            </span>
          )}
        </div>
        {/* eslint-disable-next-line jsx-a11y/no-static-element-interactions */}
        <span
          onClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => e.stopPropagation()}
          className="shrink-0 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100"
        >
          <CopyButton value={text} label={t("思考プロセスをコピー")} />
        </span>
      </summary>
      <div className="max-h-72 overflow-y-auto whitespace-pre-wrap border-t border-line px-3 py-2 text-xs leading-5 text-muted select-text">
        {text}
      </div>
    </details>
  );
});

/** Compact, inline view of the tool calls the agent made in this run with
 *  collapsible diffs and terminal execution telemetry. */
const InlineTools = memo(function InlineTools({
  tools,
}: {
  tools: ToolActivity[];
}) {
  return (
    <div className="flex flex-col gap-1.5">
      {tools.map((tool, idx) => (
        <InlineToolItem
          key={tool.id}
          tool={tool}
          defaultOpen={
            tool.status === "running" ||
            tool.status === "error" ||
            (idx === tools.length - 1 &&
              (tool.name === "edit" || tool.name === "terminal"))
          }
        />
      ))}
    </div>
  );
});

/** Compact inline checklist of the agent's active plan. */
const InlineTodoList = memo(function InlineTodoList({
  todos,
}: {
  todos: TodoItem[];
}) {
  const done = todos.filter((i) => i.status === "completed").length;
  const current =
    todos.find((i) => i.status === "in_progress") ??
    todos.find((i) => i.status === "pending");

  return (
    <details className="group overflow-hidden rounded-lg border border-line bg-surface/80 text-xs shadow-xs">
      <summary className="flex cursor-pointer list-none items-center justify-between px-2.5 py-1.5 transition-colors hover:bg-hover">
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <ListChecks size={13} className="shrink-0 text-brand" />
          <span className="font-medium text-fg">{t("計画")}</span>
          <Badge tone={done === todos.length ? "success" : "brand"} size="sm">
            {done}/{todos.length}
          </Badge>
          {current && (
            <span className="truncate text-2xs text-muted">
              {current.content}
            </span>
          )}
        </div>
        <ChevronRight
          size={12}
          className="shrink-0 text-faint transition-transform group-open:rotate-90"
        />
      </summary>
      <ul className="flex flex-col gap-1 border-t border-line p-2">
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
    </details>
  );
});

export const LiveBlock = memo(function LiveBlock({
  live,
}: {
  live: RunTimeline;
}) {
  // Waiting on an approval is still "in flight": the spinner has to stay, and
  // the transcript must not be retired while a card is waiting for an answer.
  const busy = isBusy(live.status);
  return (
    <div className="flex gap-3">
      <span
        aria-hidden="true"
        className={cn(
          "mt-0.5 grid size-6 shrink-0 place-items-center rounded-lg border border-line bg-gradient-to-b from-surface to-inset text-fg",
          busy && "animate-mix-pulse motion-reduce:animate-none",
        )}
      >
        <Sparkles size={12} />
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        {live.error && (
          <Alert tone="danger" role="status">
            <strong className="block font-medium">
              {t("実行に失敗しました")}
            </strong>
            <span className="whitespace-pre-wrap break-words">
              {live.error}
            </span>
          </Alert>
        )}
        {live.stagnation && (
          <Alert tone="warning" role="status">
            <strong className="block font-medium">
              {t("停滞を検出しました")}
            </strong>
            <span>{live.stagnation}</span>
          </Alert>
        )}
        {live.verification && (
          <Alert tone="danger" role="status">
            <strong className="block font-medium">{t("検証が必要です")}</strong>
            <span>
              {tf(
                "破壊的ツール {tool} が実行されました。確認の読み取りが期待されます。",
                {
                  tool: live.verification.tool,
                },
              )}
            </span>
          </Alert>
        )}
        {live.budgetRequest && <BudgetBanner live={live} />}
        {live.approval && <ApprovalCard live={live} />}
        {live.reasoning && (
          <Reasoning text={live.reasoning} streaming={busy && !live.text} />
        )}
        {live.todos && live.todos.length > 0 && (
          <InlineTodoList todos={live.todos} />
        )}
        {live.tools.length > 0 && <InlineTools tools={live.tools} />}
        {live.text ? (
          <div className="text-sm text-fg">
            <Markdown>{stripLeakedMarkup(live.text)}</Markdown>
          </div>
        ) : (
          busy && (
            <div className="flex items-center gap-1.5 text-xs text-faint">
              <Spinner size={12} />
              {t("思考中…")}
            </div>
          )
        )}
        {live.verificationFailed && (
          <Alert tone="warning" hideIcon role="status">
            <span className="inline-flex items-start gap-2">
              <ShieldAlert
                size={13}
                className="mt-0.5 shrink-0"
                aria-hidden="true"
              />
              <span>{live.verificationFailed}</span>
            </span>
          </Alert>
        )}
      </div>
    </div>
  );
});

function BudgetBanner({ live }: { live: RunTimeline }) {
  const request = live.budgetRequest;
  if (!request) return null;
  return (
    <Alert tone="warning" className="flex-col items-start" role="status">
      <strong className="block font-medium">{t("予算を使い切りました")}</strong>
      <span>
        {tf("ステップ上限 {steps} / ツール上限 {calls} に到達しました。", {
          steps: request.stepLimit ?? "?",
          calls: request.toolCallLimit ?? "?",
        })}{" "}
        {request.reason}
      </span>
      <div className="mt-2 flex items-center gap-2">
        <BudgetButtons runId={live.runId} request={request} />
      </div>
    </Alert>
  );
}

const BudgetButtons = memo(function BudgetButtons({
  runId,
  request,
}: {
  runId: string;
  request: { stepLimit?: number; toolCallLimit?: number };
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const decide = async (grant: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await api.budgetExtension({
        id: runId,
        grant,
        step_limit: grant ? (request.stepLimit ?? null) : null,
        tool_call_limit: grant ? (request.toolCallLimit ?? null) : null,
        wall_clock_minutes: grant ? 30 : null,
      });
    } catch (err) {
      // A rejected answer used to vanish: the buttons simply re-enabled and the
      // run stayed blocked with no explanation.
      setError(errorMessage(err) || t("エラー"));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <Button
        variant="primary"
        size="sm"
        disabled={busy}
        onClick={() => void decide(true)}
      >
        {t("延長する")}
      </Button>
      <Button size="sm" disabled={busy} onClick={() => void decide(false)}>
        {t("ここで終了する")}
      </Button>
      {error && (
        <span role="status">
          <ErrorBadge error={error} />
        </span>
      )}
    </>
  );
});
