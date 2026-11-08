import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Activity, Square } from "lucide-react";
import { api } from "@/lib/api";
import { t, tf } from "@/lib/i18n";
import { surfaceForMode, useUi } from "@/lib/ui";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { ActiveRunView } from "@/lib/types";

const STATUS_LABEL: Record<string, string> = {
  queued: "待機中",
  running: "実行中",
  waiting_approval: "承認待ち",
  budget_extension_pending: "予算延長待ち",
};

/**
 * Global inbox for live Runs.
 *
 * Before this, a background Run in another thread was only a dot in the
 * sidebar: there was no one place to see what was running, jump to it, or stop
 * it. The header chip stays out of the way when nothing is running.
 */
export function ActiveRunsIndicator() {
  const queryClient = useQueryClient();
  const setSurface = useUi((s) => s.setSurface);
  const setConversationId = useUi((s) => s.setConversationId);
  const setView = useUi((s) => s.setView);
  const setActivityRunId = useUi((s) => s.setActivityRunId);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const query = useQuery({
    queryKey: ["active-runs"],
    queryFn: api.listActiveRuns,
    refetchInterval: 5_000,
    refetchIntervalInBackground: false,
  });
  const runs = query.data ?? [];
  if (runs.length === 0) return null;

  const openRun = (run: ActiveRunView) => {
    setSurface(surfaceForMode(run.mode));
    setConversationId(run.conversation_id);
    setView("chat");
    setActivityRunId(run.run_id);
    setRightPanel("activity");
  };

  const stopRun = async (run: ActiveRunView) => {
    try {
      await api.cancelRun(run.run_id);
    } finally {
      void queryClient.invalidateQueries({ queryKey: ["active-runs"] });
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          className="relative"
          title={tf("{count} 件実行中", { count: runs.length })}
          aria-label={tf("{count} 件実行中", { count: runs.length })}
        >
          <Activity size={13} className="text-brand" aria-hidden="true" />
          <span className="absolute -top-0.5 -right-1 rounded-full bg-brand px-1 text-[9px] font-semibold leading-3 text-on-solid tabular-nums">
            {runs.length}
          </span>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-80">
        <DropdownMenuLabel>{t("実行中")}</DropdownMenuLabel>
        {runs.map((run) => (
          <div key={run.run_id} className="flex items-center gap-1 px-1">
            <DropdownMenuItem
              className="min-w-0 flex-1"
              onSelect={() => openRun(run)}
            >
              <span className="min-w-0 flex-1 truncate text-fg">
                {run.title || t("無題のスレッド")}
              </span>
              <Badge
                size="sm"
                tone={run.status === "waiting_approval" ? "warning" : "neutral"}
              >
                {t(STATUS_LABEL[run.status] ?? run.status)}
              </Badge>
              <span className="shrink-0 font-mono text-2xs text-faint">
                {tf("ステップ {step}", { step: run.step_count })}
              </span>
            </DropdownMenuItem>
            <Button
              size="icon-sm"
              variant="ghost"
              className="text-faint hover:text-danger"
              title={t("実行を停止")}
              aria-label={t("実行を停止")}
              onClick={() => void stopRun(run)}
            >
              <Square size={10} aria-hidden="true" />
            </Button>
          </div>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => setView("chat")}>
          {t("チャットに戻る")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
