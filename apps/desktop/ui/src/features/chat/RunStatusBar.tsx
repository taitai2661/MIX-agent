import { memo, useEffect, useRef, useState } from "react";
import { Activity, ShieldAlert } from "lucide-react";
import { t, tf } from "@/lib/i18n";
import { isBusy, type ToolActivity } from "@/lib/runEvents";
import { useRunEvents } from "@/lib/runStore";
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
}: {
  conversationId: string | null;
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

  const [tick, setTick] = useState(0);
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
  const label = approval
    ? t("承認待ち")
    : activeTool
      ? tf("{tool} を実行中…", { tool: activeTool.name })
      : status === "queued"
        ? t("待機中…")
        : t("実行中…");

  const elapsed =
    started.current?.runId === runId
      ? Math.max(0, Math.floor((Date.now() - started.current.at) / 1000))
      : 0;
  void tick; // re-render every second so the elapsed label advances

  return (
    <div
      className={cn(
        "flex shrink-0 items-center gap-2 border-t border-line bg-surface/70 px-4 py-1.5",
        "text-xs text-muted backdrop-blur-sm",
        approval && "bg-warning-soft text-warning",
      )}
      data-run-status={status}
    >
      {approval ? (
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
    </div>
  );
});

function formatElapsed(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return `${minutes}:${String(rest).padStart(2, "0")}`;
}
