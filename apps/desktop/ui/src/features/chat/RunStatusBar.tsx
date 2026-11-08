import { memo, useEffect, useRef, useState } from "react";
import { Activity, ShieldAlert, Square } from "lucide-react";
import { api } from "@/lib/api";
import { t, tf } from "@/lib/i18n";
import { isBusy, type ToolActivity } from "@/lib/runEvents";
import { useRunEvents } from "@/lib/runStore";
import { useUi } from "@/lib/ui";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/cn";

const EMPTY_TOOLS: ToolActivity[] = [];

/**
 * One line that answers "is it working, and on what?" without opening the
 * Activity panel.
 *
 * The sidebar only shows a dot and the transcript's live block scrolls out of
 * view on long answers. This bar sits above the composer, so the state stays
 * visible while the user reads history or types guidance. It subscribes to
 * derived slices rather than the whole timeline, so streaming tokens do not
 * re-render it.
 */
export const RunStatusBar = memo(function RunStatusBar({
  conversationId,
  startedAt = null,
}: {
  conversationId: string | null;
  /** Persisted Run start time (ISO). Passing it keeps the elapsed label
   *  correct after switching back to a live thread or relaunching the app;
   *  the locally observed time is only a fallback while the row is loading. */
  startedAt?: string | null;
}) {
  const runId = useRunEvents((s) =>
    conversationId ? (s.byConversation[conversationId]?.runId ?? null) : null,
  );
  const status = useRunEvents((s) =>
    conversationId
      ? (s.byConversation[conversationId]?.timeline.status ?? null)
      : null,
  );
  const approval = useRunEvents((s) =>
    conversationId
      ? (s.byConversation[conversationId]?.timeline.approval ?? null)
      : null,
  );
  const tools = useRunEvents((s) =>
    conversationId
      ? (s.byConversation[conversationId]?.timeline.tools ?? EMPTY_TOOLS)
      : EMPTY_TOOLS,
  );
  const budgetRequest = useRunEvents((s) =>
    conversationId
      ? (s.byConversation[conversationId]?.timeline.budgetRequest ?? null)
      : null,
  );
  // Subscribe to booleans, not the text itself: the selector result only
  // changes on the empty → non-empty transition, so streaming tokens do not
  // re-render this bar 16×/s.
  const hasText = useRunEvents((s) =>
    conversationId
      ? (s.byConversation[conversationId]?.timeline.text.length ?? 0) > 0
      : false,
  );
  const hasReasoning = useRunEvents((s) =>
    conversationId
      ? (s.byConversation[conversationId]?.timeline.reasoning.length ?? 0) > 0
      : false,
  );

  const [tick, setTick] = useState(0);
  const [stopping, setStopping] = useState(false);
  const setApprovalsOpen = useUi((s) => s.setApprovalsOpen);
  const started = useRef<{ runId: string; at: number } | null>(null);

  useEffect(() => {
    if (!runId) {
      started.current = null;
      return;
    }
    if (started.current?.runId !== runId) {
      started.current = { runId, at: Date.now() };
      setTick(0);
    }
    const timer = window.setInterval(() => setTick((value) => value + 1), 1000);
    return () => window.clearInterval(timer);
  }, [runId]);

  if (!runId || !status || !isBusy(status)) return null;

  const activeTool = [...tools]
    .reverse()
    .find((tool) => tool.status === "running");
  const waitingApproval = !!approval || status === "waiting_approval";
  const budgetPending =
    !!budgetRequest || status === "budget_extension_pending";
  // Order matters: a blocked run must say what it is waiting for, even while
  // an earlier tool is still marked "running" in the timeline.
  const label = waitingApproval
    ? t("承認待ち")
    : budgetPending
      ? t("予算延長待ち")
      : activeTool
        ? tf("{tool} を実行中…", { tool: activeTool.name })
        : status === "queued"
          ? t("待機中…")
          : hasText
            ? t("応答を生成中…")
            : hasReasoning
              ? t("考え中…")
              : t("実行中…");

  const persistedStart = startedAt ? Date.parse(startedAt) : Number.NaN;
  const elapsedFrom =
    Number.isFinite(persistedStart) && persistedStart > 0
      ? persistedStart
      : started.current?.runId === runId
        ? started.current.at
        : Date.now();
  const elapsed = Math.max(0, Math.floor((Date.now() - elapsedFrom) / 1000));
  void tick; // re-render every second so the elapsed label advances

  return (
    <div
      className={cn(
        "flex shrink-0 items-center gap-2 border-t border-line bg-surface/70 px-4 py-1.5",
        "text-xs text-muted backdrop-blur-sm",
        (waitingApproval || budgetPending) && "bg-warning-soft text-warning",
      )}
      data-run-status={status}
    >
      {waitingApproval ? (
        <ShieldAlert size={13} className="shrink-0" aria-hidden="true" />
      ) : (
        <Spinner size={12} />
      )}
      <span
        role="status"
        aria-live="polite"
        className="min-w-0 truncate font-medium"
      >
        {label}
      </span>
      <span className="flex-1" />
      <span
        aria-hidden="true"
        className="shrink-0 font-mono text-2xs text-faint tabular-nums"
        title={t("このRunの経過時間")}
      >
        {formatElapsed(elapsed)}
      </span>
      <Activity size={12} className="shrink-0 text-faint" aria-hidden="true" />
      {waitingApproval && (
        <Button
          size="sm"
          variant="default"
          className="h-6 px-2 text-2xs"
          onClick={() => setApprovalsOpen(true)}
        >
          <ShieldAlert size={11} aria-hidden="true" />
          {t("承認")}
        </Button>
      )}
      <Button
        size="icon-sm"
        variant="ghost"
        title={t("実行を停止")}
        aria-label={t("実行を停止")}
        disabled={stopping}
        onClick={async () => {
          setStopping(true);
          try {
            await api.cancelRun(runId);
          } finally {
            setStopping(false);
          }
        }}
      >
        <Square size={10} aria-hidden="true" />
      </Button>
    </div>
  );
});

function formatElapsed(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}
