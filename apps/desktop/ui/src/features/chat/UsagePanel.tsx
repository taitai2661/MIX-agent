import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Coins, Database, Gauge } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { formatCost, formatTokens, shortModelId } from "@/lib/format";
import { t } from "@/lib/i18n";
import { useRunEvents } from "@/lib/runStore";
import type { RunUsage } from "@/lib/types";
import { useUi } from "@/lib/ui";
import { Card, PANEL_BODY, PanelHeader } from "@/components/layout";
import { Badge } from "@/components/ui/badge";
import { EmptyState, ErrorBox, ListSkeleton } from "@/components/shared";

const EMPTY_USAGE: RunUsage = {
  input_tokens: 0,
  output_tokens: 0,
  cache_read_tokens: 0,
  cache_write_tokens: 0,
  context_tokens: 0,
  context_window: 0,
  estimated_cost_usd: null,
};

/** Fold several Run summaries into one. Context is the newest turn's prompt
 *  size, not a sum, so the last non-zero value wins — mirroring the engine,
 *  which reports `context_tokens` per turn while tokens are cumulative. */
function addUsage(
  total: RunUsage,
  next: RunUsage | null | undefined,
): RunUsage {
  if (!next) return total;
  const cost =
    next.estimated_cost_usd == null
      ? total.estimated_cost_usd
      : (total.estimated_cost_usd ?? 0) + next.estimated_cost_usd;
  return {
    input_tokens: total.input_tokens + next.input_tokens,
    output_tokens: total.output_tokens + next.output_tokens,
    cache_read_tokens: total.cache_read_tokens + next.cache_read_tokens,
    cache_write_tokens: total.cache_write_tokens + next.cache_write_tokens,
    context_tokens: next.context_tokens || total.context_tokens,
    context_window: next.context_window || total.context_window,
    estimated_cost_usd: cost,
  };
}

function contextPercent(usage: RunUsage | null): number | null {
  if (!usage || usage.context_window <= 0) return null;
  const percent = Math.round(
    (usage.context_tokens / usage.context_window) * 100,
  );
  return Math.max(0, Math.min(100, percent));
}

function StatRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-xs">
      <span className="min-w-0 truncate text-muted">{label}</span>
      <span className="shrink-0 font-mono tabular-nums text-fg">{value}</span>
    </div>
  );
}

/** A small SVG ring plus the percentage in the middle. */
function ContextRing({ percent }: { percent: number | null }) {
  const radius = 32;
  const circumference = 2 * Math.PI * radius;
  const filled = percent == null ? 0 : (percent / 100) * circumference;
  const tone =
    percent == null
      ? "text-faint"
      : percent >= 90
        ? "text-danger"
        : percent >= 75
          ? "text-warning"
          : "text-brand";

  return (
    <div className="relative grid size-20 shrink-0 place-items-center">
      <svg
        className="absolute inset-0 -rotate-90"
        viewBox="0 0 80 80"
        aria-hidden="true"
      >
        <circle
          cx="40"
          cy="40"
          r={radius}
          fill="none"
          strokeWidth="6"
          className="stroke-inset"
        />
        <circle
          cx="40"
          cy="40"
          r={radius}
          fill="none"
          strokeWidth="6"
          strokeLinecap="round"
          stroke="currentColor"
          strokeDasharray={circumference}
          strokeDashoffset={circumference - filled}
          className={cn("transition-[stroke-dashoffset] duration-300", tone)}
        />
      </svg>
      <span className="text-base font-semibold tabular-nums text-fg">
        {percent == null ? "—" : `${percent}%`}
      </span>
    </div>
  );
}

/** Right-panel usage view: context window, token totals and estimated spend.
 *
 *  Reads the live timeline when the selected Run is streaming and falls back
 *  to the persisted `usage` summary on `runs` rows, so the panel keeps showing
 *  the same numbers after a reload or once the Run has finished.
 */
export function UsagePanel({ embedded = false }: { embedded?: boolean } = {}) {
  const conversationId = useUi((s) => s.conversationId);
  const activityRunId = useUi((s) => s.activityRunId);
  const setRightPanel = useUi((s) => s.setRightPanel);

  const liveEntry = useRunEvents((s) =>
    conversationId ? s.byConversation[conversationId] : undefined,
  );
  const liveRunId = liveEntry?.runId ?? null;
  const liveUsage = liveEntry?.timeline.usage ?? null;

  const runs = useQuery({
    queryKey: ["runs", conversationId],
    queryFn: () => api.listRuns(conversationId!),
    enabled: !!conversationId,
    staleTime: Infinity,
  });

  const rows = useMemo(() => runs.data ?? [], [runs.data]);
  // `list_runs` orders oldest → newest, so the tail is the latest Run.
  const selectedRunId = activityRunId ?? liveRunId ?? rows.at(-1)?.id ?? null;
  const selectedRun = rows.find((run) => run.id === selectedRunId) ?? null;
  const currentUsage =
    selectedRunId != null && selectedRunId === liveRunId
      ? (liveUsage ?? selectedRun?.usage ?? null)
      : (selectedRun?.usage ?? null);

  const conversationUsage = useMemo(() => {
    let total = EMPTY_USAGE;
    for (const run of rows) {
      total = addUsage(
        total,
        run.id === liveRunId && liveUsage ? liveUsage : run.usage,
      );
    }
    // A just-started Run may not be in `list_runs` yet. Count it once here so
    // the conversation total matches the live card instead of lagging behind.
    if (currentUsage && !rows.some((run) => run.id === selectedRunId)) {
      total = addUsage(total, currentUsage);
    }
    return total;
  }, [rows, liveRunId, liveUsage, currentUsage, selectedRunId]);

  const usage = currentUsage ?? (runs.data ? conversationUsage : null);
  const percent = contextPercent(usage);
  const model = selectedRun?.model_id ?? null;
  const hasUsage =
    currentUsage != null ||
    conversationUsage.input_tokens > 0 ||
    conversationUsage.output_tokens > 0 ||
    conversationUsage.estimated_cost_usd != null;

  return (
    <>
      {!embedded && (
        <PanelHeader
          icon={<Gauge size={14} />}
          title={t("使用状況")}
          trailing={
            percent != null ? <Badge size="sm">{percent}%</Badge> : null
          }
          onClose={() => setRightPanel("none")}
        />
      )}

      <div className={PANEL_BODY}>
        {!conversationId && (
          <EmptyState
            compact
            icon={<Gauge size={22} />}
            title={t("このスレッドにはまだ使用状況がありません")}
          />
        )}
        {conversationId && runs.isPending && !currentUsage && (
          <ListSkeleton rows={2} />
        )}
        {conversationId && runs.isError && <ErrorBox error={runs.error} />}
        {conversationId && runs.data && !hasUsage && (
          <EmptyState
            compact
            icon={<Gauge size={22} />}
            title={t("このスレッドにはまだ使用状況がありません")}
            sub={t(
              "メッセージを送ると、トークン数と推定コストが表示されます。",
            )}
          />
        )}

        {usage && (
          <Card
            title={t("コンテキスト")}
            icon={<Gauge size={13} />}
            action={
              percent != null ? (
                <Badge
                  size="sm"
                  tone={
                    percent >= 90
                      ? "danger"
                      : percent >= 75
                        ? "warning"
                        : "brand"
                  }
                >
                  {percent}%
                </Badge>
              ) : null
            }
          >
            <div className="flex items-center gap-4">
              <ContextRing percent={percent} />
              <div className="flex min-w-0 flex-1 flex-col gap-1 text-xs">
                <span className="flex items-baseline gap-1">
                  <span className="text-lg font-semibold tabular-nums text-fg">
                    {formatTokens(usage.context_tokens)}
                  </span>
                  <span className="text-faint">
                    /{" "}
                    {usage.context_window > 0
                      ? formatTokens(usage.context_window)
                      : "—"}
                  </span>
                </span>
                <span className="text-muted">
                  {t("最新ターンで使用したプロンプト量です。")}
                </span>
              </div>
            </div>
          </Card>
        )}

        {currentUsage && (
          <Card
            title={t("このRun")}
            icon={<Database size={13} />}
            action={
              model ? (
                <span
                  className="max-w-40 truncate font-mono text-xs text-muted"
                  title={model}
                >
                  {shortModelId(model)}
                </span>
              ) : null
            }
          >
            <StatRow
              label={t("入力トークン")}
              value={formatTokens(currentUsage.input_tokens)}
            />
            <StatRow
              label={t("出力トークン")}
              value={formatTokens(currentUsage.output_tokens)}
            />
            <StatRow
              label={t("キャッシュ読取")}
              value={formatTokens(currentUsage.cache_read_tokens)}
            />
            <StatRow
              label={t("キャッシュ書込")}
              value={formatTokens(currentUsage.cache_write_tokens)}
            />
            <StatRow
              label={t("推定コスト")}
              value={formatCost(currentUsage.estimated_cost_usd)}
            />
          </Card>
        )}

        {hasUsage && (
          <Card title={t("会話合計")} icon={<Coins size={13} />}>
            <StatRow
              label={t("入力トークン")}
              value={formatTokens(conversationUsage.input_tokens)}
            />
            <StatRow
              label={t("出力トークン")}
              value={formatTokens(conversationUsage.output_tokens)}
            />
            <StatRow
              label={t("キャッシュ読取")}
              value={formatTokens(conversationUsage.cache_read_tokens)}
            />
            <StatRow
              label={t("キャッシュ書込")}
              value={formatTokens(conversationUsage.cache_write_tokens)}
            />
            <StatRow
              label={t("推定コスト")}
              value={formatCost(conversationUsage.estimated_cost_usd)}
            />
          </Card>
        )}
      </div>
    </>
  );
}
