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
  Check,
  ChevronRight,
  CircleAlert,
  ShieldAlert,
  Sparkles,
  Wrench,
} from "lucide-react";
import { Markdown } from "@/components/Markdown";
import { stripLeakedMarkup } from "@/lib/format";
import { t, tf } from "@/lib/i18n";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { api } from "@/lib/api";
import {
  isBusy,
  describeToolCall,
  type RunTimeline,
  type ToolActivity,
} from "@/lib/runEvents";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
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
      className="group rounded-lg border border-line bg-inset/60"
    >
      {/* `list-none` removes the native marker, so the disclosure state needs a
          affordance of its own: a chevron that rotates with `open`. */}
      <summary className="flex cursor-pointer list-none items-center gap-1.5 rounded-lg px-3 py-2 text-xs font-medium text-muted transition-colors hover:bg-hover hover:text-fg">
        <ChevronRight
          size={12}
          aria-hidden="true"
          className="shrink-0 transition-transform duration-150 group-open:rotate-90"
        />
        <Brain size={12} className="shrink-0" aria-hidden="true" />
        {t("思考プロセス")}
      </summary>
      <div className="max-h-72 overflow-y-auto whitespace-pre-wrap border-t border-line px-3 py-2 text-xs leading-5 text-muted">
        {text}
      </div>
    </details>
  );
});

/** Compact, inline view of the tool calls the agent made in this run. The
 *  right-hand Activity panel has the full arguments/results; this keeps the
 *  conversation itself honest about what is happening while it streams. */
const InlineTools = memo(function InlineTools({
  tools,
}: {
  tools: ToolActivity[];
}) {
  return (
    <div className="flex flex-col overflow-hidden rounded-lg border border-line bg-surface shadow-xs">
      {tools.map((tool) => (
        <div
          key={tool.id}
          className="flex items-center gap-2 border-b border-line px-2.5 py-1.5 text-xs last:border-b-0"
        >
          <Wrench
            size={12}
            className="shrink-0 text-faint"
            aria-hidden="true"
          />
          <span className="shrink-0 font-mono font-medium text-fg">
            {tool.name}
          </span>
          <span
            className="min-w-0 flex-1 truncate text-muted"
            title={describeToolCall(tool)}
          >
            {describeToolCall(tool)}
          </span>
          {tool.status === "running" ? (
            <Spinner size={11} />
          ) : tool.status === "error" ? (
            <CircleAlert
              size={12}
              className="shrink-0 text-danger"
              aria-hidden="true"
            />
          ) : (
            <Check
              size={12}
              className="shrink-0 text-success"
              aria-hidden="true"
            />
          )}
        </div>
      ))}
    </div>
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
