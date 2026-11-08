import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { History, RotateCcw, X } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { surfaceForMode, useUi } from "@/lib/ui";
import { useRunEvents } from "@/lib/runStore";
import { Button } from "@/components/ui/button";
import { ErrorBox } from "@/components/shared";
import type { InterruptedRunView } from "@/lib/types";

/** Runs the user chose to leave alone for now. Session-scoped on purpose: a
 *  fresh launch should still surface work that is genuinely unfinished. */
const dismissed = new Set<string>();

/**
 * Startup recovery inbox.
 *
 * Crash-consistent runs were already marked `interrupted` in Rust, but the
 * only way to resume was to guess the right thread, open Activity and find the
 * checkpoint. This banner puts the interrupted Run(s) in front of the user with
 * a one-click resume.
 */
export function RecoveryBanner() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["interrupted-runs"],
    queryFn: api.listInterruptedRuns,
    staleTime: 0,
    retry: false,
  });
  const [busyId, setBusyId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hidden, setHidden] = useState(false);

  const runs = (query.data ?? []).filter((run) => !dismissed.has(run.run_id));
  if (hidden || runs.length === 0) return null;

  const resume = async (run: InterruptedRunView) => {
    if (!run.checkpoint_id) return;
    setBusyId(run.run_id);
    setError(null);
    try {
      await api.resumeRun({
        id: run.run_id,
        from_checkpoint: run.checkpoint_id,
        unknown_actions: [],
      });
      useRunEvents.getState().startRun(run.run_id, run.conversation_id);
      const ui = useUi.getState();
      ui.setSurface(surfaceForMode(run.mode));
      ui.setConversationId(run.conversation_id);
      ui.setView("chat");
      dismissed.add(run.run_id);
      void queryClient.invalidateQueries({ queryKey: ["interrupted-runs"] });
      void queryClient.invalidateQueries({
        queryKey: ["runs", run.conversation_id],
      });
      void queryClient.invalidateQueries({
        queryKey: ["messages", run.conversation_id],
      });
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusyId(null);
    }
  };

  const openActivity = (run: InterruptedRunView) => {
    const ui = useUi.getState();
    ui.setSurface(surfaceForMode(run.mode));
    ui.setConversationId(run.conversation_id);
    ui.setView("chat");
    ui.setActivityRunId(run.run_id);
    ui.setRightPanel("activity");
    dismissed.add(run.run_id);
    setHidden(true);
  };

  return (
    <div
      role="status"
      className="flex shrink-0 flex-col gap-1.5 border-b border-warning/25 bg-warning-soft px-4 py-2 text-xs text-warning"
    >
      <div className="flex items-center gap-2">
        <History size={13} aria-hidden="true" className="shrink-0" />
        <span className="font-medium">
          {tf("{count} 件の実行が中断されています", { count: runs.length })}
        </span>
        <span className="flex-1" />
        <Button
          size="icon-sm"
          variant="ghost"
          className="text-warning hover:bg-warning/10 hover:text-warning"
          title={t("閉じる")}
          aria-label={t("閉じる")}
          onClick={() => setHidden(true)}
        >
          <X size={12} />
        </Button>
      </div>
      {runs.slice(0, 3).map((run) => (
        <div
          key={run.run_id}
          className="flex min-w-0 items-center gap-2 rounded-md border border-warning/20 bg-bg/60 px-2 py-1"
        >
          <span className="min-w-0 flex-1 truncate text-fg">
            {run.title || t("無題のスレッド")}
          </span>
          <span className="shrink-0 font-mono text-2xs text-faint">
            {run.checkpoint_step != null
              ? tf("ステップ {step}", { step: run.checkpoint_step })
              : tf("ステップ {step}", { step: run.step_count })}
          </span>
          <Button
            size="sm"
            variant="default"
            disabled={busyId !== null || !run.checkpoint_id}
            title={
              run.checkpoint_id
                ? t("チェックポイントから再開")
                : t("再開できるチェックポイントがありません")
            }
            onClick={() => void resume(run)}
          >
            <RotateCcw size={12} />
            {busyId === run.run_id ? t("再開中…") : t("再開")}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => openActivity(run)}>
            {t("アクティビティ")}
          </Button>
        </div>
      ))}
      {runs.length > 3 && (
        <span className="text-2xs text-faint">
          {tf("ほか {count} 件", { count: runs.length - 3 })}
        </span>
      )}
      {error && <ErrorBox error={error} variant="warning" />}
    </div>
  );
}
