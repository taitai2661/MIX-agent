import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Activity,
  Coins,
  Cpu,
  Database,
  Gauge,
  History,
  Zap,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import {
  absoluteTime,
  formatBytes,
  formatContext,
  formatCost,
  formatDuration,
  formatTokens,
  relativeTime,
  shortId,
  shortModelId,
} from "@/lib/format";
import { t, tf } from "@/lib/i18n";
import { useConversation, useModels, useProviders } from "@/lib/queries";
import { useRunEvents } from "@/lib/runStore";
import type { RunUsage, RunView } from "@/lib/types";
import { useUi } from "@/lib/ui";
import { Card, PANEL_BODY, PanelHeader } from "@/components/layout";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { EmptyState, ErrorBox, ListSkeleton } from "@/components/shared";
import { ALL_MODES } from "./composerModes";

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

/** Billed tokens across every bucket (input + output + cache). */
function totalTokens(usage: RunUsage): number {
  return (
    usage.input_tokens +
    usage.output_tokens +
    usage.cache_read_tokens +
    usage.cache_write_tokens
  );
}

/** Share of prompt tokens served from the provider's cache. */
function cacheHitPercent(usage: RunUsage): number | null {
  const prompt =
    usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens;
  if (prompt <= 0) return null;
  return Math.round((usage.cache_read_tokens / prompt) * 100);
}

/** `used / limit` when the Run has a ceiling, otherwise just `used`. */
function budgetValue(used: number, limit: number | undefined): string {
  return limit && limit > 0
    ? `${formatTokens(used)} / ${formatTokens(limit)}`
    : formatTokens(used);
}

function runDurationMs(run: RunView, live: boolean): number {
  const start = new Date(run.started_at).getTime();
  const end = live ? Date.now() : new Date(run.updated_at).getTime();
  if (!Number.isFinite(start) || !Number.isFinite(end)) return 0;
  return Math.max(0, end - start);
}

/** Tokens per second, with just enough precision to compare Runs. */
function formatRate(tokens: number, durationMs: number): string {
  if (!Number.isFinite(durationMs) || durationMs <= 0 || tokens <= 0) {
    return "—";
  }
  const rate = tokens / (durationMs / 1000);
  if (rate >= 100) return rate.toFixed(0);
  if (rate >= 10) return rate.toFixed(1);
  return rate.toFixed(2);
}

const RUN_STATUS_LABELS: Record<string, string> = {
  queued: "待機中",
  running: "実行中",
  waiting_approval: "承認待ち",
  budget_extension_pending: "予算延長待ち",
  completed: "完了",
  failed: "失敗",
  cancelled: "キャンセル済み",
  interrupted: "中断済み",
};

function statusTone(status: string): NonNullable<BadgeProps["tone"]> {
  if (status === "completed") return "success";
  if (status === "failed") return "danger";
  if (
    status === "queued" ||
    status === "running" ||
    status === "waiting_approval" ||
    status === "budget_extension_pending"
  ) {
    return "warning";
  }
  return "neutral";
}

function StatRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-xs">
      <span className="min-w-0 truncate text-muted">{label}</span>
      <span
        className="max-w-[65%] truncate font-mono tabular-nums text-fg"
        title={value}
      >
        {value}
      </span>
    </div>
  );
}

/** A capability flag with a visible on/off state. */
function CapabilityBadge({ label, on }: { label: string; on: boolean }) {
  return (
    <Badge tone={on ? "success" : "neutral"} size="sm">
      {label}
      <span aria-hidden="true">{on ? "✓" : "—"}</span>
      <span className="sr-only">{on ? t("対応") : t("非対応")}</span>
    </Badge>
  );
}

function runStatusLabel(status: string | null): string | null {
  if (!status) return null;
  return t(RUN_STATUS_LABELS[status] ?? status);
}

/** Proportional bar with a compact legend, so the four token buckets are
 *  readable at a glance instead of only as four separate rows. */
function TokenBreakdown({ usage }: { usage: RunUsage }) {
  const total = totalTokens(usage);
  if (total <= 0) return null;
  const parts = [
    { label: t("入力トークン"), value: usage.input_tokens, color: "bg-brand" },
    {
      label: t("出力トークン"),
      value: usage.output_tokens,
      color: "bg-success",
    },
    {
      label: t("キャッシュ読取"),
      value: usage.cache_read_tokens,
      color: "bg-info",
    },
    {
      label: t("キャッシュ書込"),
      value: usage.cache_write_tokens,
      color: "bg-warning",
    },
  ].filter((part) => part.value > 0);

  return (
    <div className="flex flex-col gap-1.5">
      <div
        className="flex h-1.5 overflow-hidden rounded-full bg-inset"
        aria-hidden="true"
      >
        {parts.map((part) => (
          <span
            key={part.label}
            className={cn("h-full", part.color)}
            style={{ width: `${(part.value / total) * 100}%` }}
          />
        ))}
      </div>
      <div className="flex flex-wrap gap-x-2 gap-y-0.5 text-2xs text-faint">
        {parts.map((part) => (
          <span key={part.label} className="inline-flex items-center gap-1">
            <span className={cn("size-1.5 rounded-full", part.color)} />
            {part.label} {Math.round((part.value / total) * 100)}%
          </span>
        ))}
      </div>
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
  const setActivityRunId = useUi((s) => s.setActivityRunId);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const models = useModels();
  const providers = useProviders();
  const conversation = useConversation(conversationId);

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
  const hasUsage =
    currentUsage != null ||
    conversationUsage.input_tokens > 0 ||
    conversationUsage.output_tokens > 0 ||
    conversationUsage.estimated_cost_usd != null;

  const isSelectedLive = selectedRunId != null && selectedRunId === liveRunId;
  // A live Run's persisted row lags the event stream, so prefer the timeline's
  // status while it is streaming.
  const runStatus = isSelectedLive
    ? (liveEntry?.timeline.status ?? selectedRun?.status ?? null)
    : (selectedRun?.status ?? null);

  // Fall back to the conversation's model for a thread that has no Run yet —
  // the panel still has useful model details before the first message.
  const modelId =
    selectedRun?.model_id ??
    (conversation.data?.model_id !== "auto"
      ? conversation.data?.model_id
      : null) ??
    null;
  const modelView = useMemo(
    () =>
      modelId ? (models.data?.find((m) => m.id === modelId) ?? null) : null,
    [modelId, models.data],
  );
  const providerId =
    modelView?.provider_id ??
    (modelId?.includes(":") ? modelId.slice(0, modelId.indexOf(":")) : null);
  const provider = providers.data?.find((p) => p.id === providerId) ?? null;
  const modelLabel = modelView?.label ?? (modelId ? shortModelId(modelId) : "");
  const modeLabel =
    ALL_MODES.find((mode) => mode.id === selectedRun?.mode)?.label ??
    selectedRun?.mode ??
    "";
  const selectedUsage = currentUsage ?? selectedRun?.usage ?? null;
  // The engine's token budget counts input + output only (cache buckets are
  // excluded), so match that here instead of the billed total.
  const budgetTokensUsed = selectedUsage
    ? selectedUsage.input_tokens + selectedUsage.output_tokens
    : 0;
  const budget = selectedRun?.budget ?? null;
  const cacheHit = usage ? cacheHitPercent(usage) : null;
  const conversationCacheHit = cacheHitPercent(conversationUsage);
  const runCount =
    rows.length +
    (currentUsage && !rows.some((run) => run.id === selectedRunId) ? 1 : 0);
  const contextRemaining =
    usage && usage.context_window > 0
      ? Math.max(0, usage.context_window - usage.context_tokens)
      : null;
  const averageCost =
    runCount > 0 && conversationUsage.estimated_cost_usd != null
      ? conversationUsage.estimated_cost_usd / runCount
      : null;
  const selectedDuration = selectedRun
    ? runDurationMs(selectedRun, isSelectedLive)
    : 0;
  const outputRate = selectedUsage
    ? formatRate(selectedUsage.output_tokens, selectedDuration)
    : "—";
  const totalRate = selectedUsage
    ? formatRate(totalTokens(selectedUsage), selectedDuration)
    : "—";
  const msPerStep =
    selectedRun && selectedRun.step_count > 0
      ? selectedDuration / selectedRun.step_count
      : null;
  const msPerTool =
    selectedRun && selectedRun.tool_call_count > 0
      ? selectedDuration / selectedRun.tool_call_count
      : null;
  const costPerMinute =
    selectedUsage?.estimated_cost_usd != null && selectedDuration > 0
      ? selectedUsage.estimated_cost_usd / (selectedDuration / 60_000)
      : null;
  const conversationDuration = rows.reduce(
    (sum, run) => sum + runDurationMs(run, run.id === liveRunId),
    0,
  );

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
                  {contextRemaining != null && (
                    <>
                      {" · "}
                      {t("残り")} {formatTokens(contextRemaining)}
                    </>
                  )}
                </span>
              </div>
            </div>
          </Card>
        )}

        {modelId && (
          <Card
            title={t("モデル詳細")}
            icon={<Cpu size={13} />}
            action={
              provider ? (
                <Badge
                  size="sm"
                  className="max-w-28 truncate"
                  title={provider.label}
                >
                  {provider.label}
                </Badge>
              ) : null
            }
          >
            <div className="flex min-w-0 flex-col gap-0.5">
              <span
                className="truncate text-sm font-semibold text-fg"
                title={modelLabel}
              >
                {modelLabel}
              </span>
              <span
                className="break-all font-mono text-2xs text-faint"
                title={modelId}
              >
                {shortModelId(modelId)}
              </span>
            </div>
            {modelView && (
              <>
                <div className="flex flex-wrap items-center gap-1">
                  <CapabilityBadge
                    label={t("Vision")}
                    on={modelView.supports_vision}
                  />
                  <CapabilityBadge
                    label={t("ツール")}
                    on={modelView.supports_tool_calling}
                  />
                  <CapabilityBadge
                    label={t("推論")}
                    on={modelView.supports_reasoning}
                  />
                  {modelView.loaded && (
                    <Badge tone="success" size="sm">
                      {t("ロード中")}
                    </Badge>
                  )}
                </div>
                <div className="flex flex-col gap-1">
                  <StatRow
                    label={t("コンテキストウィンドウ")}
                    value={formatContext(modelView.context_window)}
                  />
                  <StatRow
                    label={t("Auto候補")}
                    value={modelView.auto_candidate ? t("対象") : t("対象外")}
                  />
                  {provider && (
                    <>
                      <StatRow label={t("種別")} value={provider.kind} />
                      {provider.base_url && (
                        <StatRow
                          label={t("接続先")}
                          value={provider.base_url}
                        />
                      )}
                    </>
                  )}
                  {modelView.family && (
                    <StatRow label={t("ファミリー")} value={modelView.family} />
                  )}
                  {modelView.parameter_size && (
                    <StatRow
                      label={t("パラメータ数")}
                      value={modelView.parameter_size}
                    />
                  )}
                  {modelView.quantization && (
                    <StatRow
                      label={t("量子化")}
                      value={modelView.quantization}
                    />
                  )}
                  {(modelView.size_bytes ?? 0) > 0 && (
                    <StatRow
                      label={t("サイズ")}
                      value={formatBytes(modelView.size_bytes ?? 0)}
                    />
                  )}
                </div>
              </>
            )}
          </Card>
        )}

        {currentUsage && (
          <Card
            title={t("このRun")}
            icon={<Database size={13} />}
            action={
              modelId ? (
                <span
                  className="max-w-40 truncate font-mono text-xs text-muted"
                  title={modelId}
                >
                  {shortModelId(modelId)}
                </span>
              ) : null
            }
          >
            <TokenBreakdown usage={currentUsage} />
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
              label={t("合計トークン")}
              value={formatTokens(totalTokens(currentUsage))}
            />
            {cacheHit != null && (
              <StatRow label={t("キャッシュヒット率")} value={`${cacheHit}%`} />
            )}
            <StatRow
              label={t("推定コスト")}
              value={formatCost(currentUsage.estimated_cost_usd)}
            />
          </Card>
        )}

        {selectedRun && (
          <Card
            title={t("Runの詳細")}
            icon={<Activity size={13} />}
            action={
              runStatus ? (
                <Badge tone={statusTone(runStatus)} size="sm">
                  {runStatusLabel(runStatus)}
                </Badge>
              ) : null
            }
          >
            <StatRow label={t("モード")} value={t(modeLabel)} />
            <StatRow
              label={t("ステップ数")}
              value={budgetValue(selectedRun.step_count, budget?.stepLimit)}
            />
            <StatRow
              label={t("ツール呼び出し")}
              value={budgetValue(
                selectedRun.tool_call_count,
                budget?.toolCallLimit,
              )}
            />
            {(selectedRun.failed_tool_count ?? 0) > 0 && (
              <StatRow
                label={t("失敗したツール呼び出し")}
                value={String(selectedRun.failed_tool_count)}
              />
            )}
            {(budget?.tokenLimit ?? 0) > 0 && (
              <>
                <StatRow
                  label={t("トークン上限")}
                  value={budgetValue(budgetTokensUsed, budget?.tokenLimit)}
                />
                <StatRow
                  label={t("残りトークン")}
                  value={formatTokens(
                    Math.max(0, (budget?.tokenLimit ?? 0) - budgetTokensUsed),
                  )}
                />
              </>
            )}
            {(budget?.wallClockMinutes ?? 0) > 0 && (
              <StatRow
                label={t("時間上限")}
                value={formatDuration((budget?.wallClockMinutes ?? 0) * 60_000)}
              />
            )}
            {(budget?.maxGrants ?? 0) > 0 && (
              <StatRow
                label={t("予算延長")}
                value={`${budget?.grantsUsed ?? 0} / ${budget?.maxGrants ?? 0}`}
              />
            )}
            <StatRow
              label={t("所要時間")}
              value={formatDuration(selectedDuration)}
            />
            <StatRow
              label={t("開始")}
              value={absoluteTime(selectedRun.started_at)}
            />
            <StatRow label={t("Run ID")} value={shortId(selectedRun.id)} />
          </Card>
        )}

        {selectedRun && selectedUsage && (
          <Card title={t("速度")} icon={<Zap size={13} />}>
            <StatRow label={t("平均出力速度")} value={`${outputRate} tok/s`} />
            <StatRow label={t("平均合計速度")} value={`${totalRate} tok/s`} />
            {msPerStep != null && (
              <StatRow
                label={t("1ステップあたり")}
                value={formatDuration(msPerStep)}
              />
            )}
            {msPerTool != null && (
              <StatRow
                label={t("1ツール呼び出しあたり")}
                value={formatDuration(msPerTool)}
              />
            )}
            {costPerMinute != null && (
              <StatRow
                label={t("コスト/分")}
                value={formatCost(costPerMinute)}
              />
            )}
            <p className="m-0 text-2xs text-muted">
              {t("ツール実行や待ち時間を含む、Run全体の平均です。")}
            </p>
          </Card>
        )}

        {rows.length > 0 && (
          <Card
            title={t("Run履歴")}
            icon={<History size={13} />}
            action={<Badge size="sm">{rows.length}</Badge>}
            bodyClassName="gap-0 p-0"
          >
            <ul className="flex max-h-64 flex-col divide-y divide-line overflow-y-auto">
              {[...rows].reverse().map((run) => {
                const selected = run.id === selectedRunId;
                const runDuration = runDurationMs(run, run.id === liveRunId);
                return (
                  <li key={run.id}>
                    <button
                      type="button"
                      aria-pressed={selected}
                      onClick={() => setActivityRunId(run.id)}
                      className={cn(
                        "flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-hover",
                        selected && "bg-brand-soft",
                      )}
                    >
                      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                        <span className="flex items-center gap-1.5">
                          <span
                            className="truncate font-mono text-xs text-fg"
                            title={run.model_id}
                          >
                            {shortModelId(run.model_id)}
                          </span>
                          <Badge tone={statusTone(run.status)} size="sm">
                            {runStatusLabel(run.status)}
                          </Badge>
                        </span>
                        <span className="text-2xs text-faint">
                          {relativeTime(run.updated_at)}
                          {" · "}
                          {tf("{n} ステップ", { n: run.step_count })}
                          {" · "}
                          {tf("{n} ツール", { n: run.tool_call_count })}
                          {" · "}
                          {formatRate(
                            run.usage?.output_tokens ?? 0,
                            runDuration,
                          )}{" "}
                          tok/s
                        </span>
                      </span>
                      <span className="shrink-0 text-right text-2xs">
                        <span className="block font-mono tabular-nums text-fg">
                          {formatTokens(run.usage ? totalTokens(run.usage) : 0)}
                        </span>
                        <span className="block text-faint">
                          {formatCost(run.usage?.estimated_cost_usd)}
                        </span>
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </Card>
        )}

        {hasUsage && (
          <Card
            title={t("会話合計")}
            icon={<Coins size={13} />}
            action={
              <span className="text-2xs text-faint">
                {tf("{n} Run", { n: runCount })}
              </span>
            }
          >
            <TokenBreakdown usage={conversationUsage} />
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
              label={t("合計トークン")}
              value={formatTokens(totalTokens(conversationUsage))}
            />
            {conversationCacheHit != null && (
              <StatRow
                label={t("キャッシュヒット率")}
                value={`${conversationCacheHit}%`}
              />
            )}
            <StatRow
              label={t("推定コスト")}
              value={formatCost(conversationUsage.estimated_cost_usd)}
            />
            {averageCost != null && (
              <StatRow
                label={t("平均コスト")}
                value={formatCost(averageCost)}
              />
            )}
            {conversationDuration > 0 &&
              conversationUsage.output_tokens > 0 && (
                <StatRow
                  label={t("平均出力速度")}
                  value={`${formatRate(
                    conversationUsage.output_tokens,
                    conversationDuration,
                  )} tok/s`}
                />
              )}
          </Card>
        )}
      </div>
    </>
  );
}
