import { useCallback, useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  ExternalLink,
  Play,
  Power,
  RefreshCw,
  RotateCw,
  ServerCog,
  Square,
  Terminal,
  Trash2,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { formatBytes, formatContext } from "@/lib/format";
import { t, tf } from "@/lib/i18n";
import { Card, Field, Row, Section } from "@/components/layout";
import { ErrorBox } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import {
  LOCAL_JOBS_QUERY_KEY,
  LOCAL_RUNTIMES_QUERY_KEY,
  MODELS_QUERY_KEY,
  PROVIDERS_QUERY_KEY,
  useLocalRuntimes,
} from "@/lib/queries";
import type { LocalJobView, LocalRuntimeSnapshotView } from "@/lib/types";

function stateTone(
  state: LocalRuntimeSnapshotView["state"],
): "neutral" | "success" | "warning" | "danger" | "info" {
  switch (state) {
    case "online":
      return "success";
    case "degraded":
    case "starting":
    case "stopping":
      return "warning";
    case "error":
      return "danger";
    case "not_installed":
      return "neutral";
    default:
      return "info";
  }
}

function stateLabel(state: LocalRuntimeSnapshotView["state"]): string {
  switch (state) {
    case "online":
      return t("オンライン");
    case "degraded":
      return t("不安定");
    case "starting":
      return t("起動中…");
    case "stopping":
      return t("停止中…");
    case "stopped":
      return t("オフライン");
    case "not_installed":
      return t("未インストール");
    case "error":
      return t("エラー");
    default:
      return t("不明");
  }
}

function jobStatusTone(
  status: string,
): "neutral" | "success" | "danger" | "info" {
  if (status === "success") return "success";
  if (status === "error") return "danger";
  if (status === "running" || status === "cancelling") return "info";
  return "neutral";
}

function jobStatusLabel(status: string): string {
  switch (status) {
    case "running":
      return t("実行中");
    case "success":
      return t("完了");
    case "error":
      return t("失敗");
    case "cancelling":
      return t("キャンセル中");
    case "cancelled":
      return t("キャンセル済み");
    default:
      return status;
  }
}

type ActionInput =
  | { op: "start" | "stop" | "restart"; slug: string }
  | { op: "delete" | "load" | "unload"; slug: string; model: string };

export function LocalRuntimePanel() {
  const queryClient = useQueryClient();
  const runtimes = useLocalRuntimes();
  const jobs = useQuery({
    queryKey: LOCAL_JOBS_QUERY_KEY,
    queryFn: api.localJobList,
    // Progress arrives as events; polling is only a fallback for a missed
    // event. One query per second while idle bought nothing.
    refetchInterval: (query) => {
      const active = (query.state.data ?? []).some(
        (job) => job.status === "running" || job.status === "cancelling",
      );
      return active ? 1000 : 10_000;
    },
  });
  const [error, setError] = useState<string | null>(null);

  const invalidate = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: LOCAL_RUNTIMES_QUERY_KEY });
    void queryClient.invalidateQueries({ queryKey: LOCAL_JOBS_QUERY_KEY });
    void queryClient.invalidateQueries({ queryKey: MODELS_QUERY_KEY });
    void queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY });
  }, [queryClient]);

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    let cancelled = false;
    // Register each listener as soon as it resolves. `await`-ing both before
    // storing either meant a rejection on the second one leaked the first, and
    // the rejection itself surfaced as an unhandled promise (not every host
    // exposes the Tauri event bridge, e.g. the web preview).
    const attach = async (
      event: string,
      handler: () => void | Promise<void>,
    ) => {
      try {
        const unlisten = await listen(event, handler);
        if (cancelled) unlisten();
        else unlisteners.push(unlisten);
      } catch (err) {
        console.warn(`could not subscribe to ${event}`, err);
      }
    };
    void attach("local_runtime_event", () => invalidate());
    void attach("local_job_event", () =>
      queryClient.invalidateQueries({ queryKey: LOCAL_JOBS_QUERY_KEY }),
    );
    return () => {
      cancelled = true;
      for (const unlisten of unlisteners) unlisten();
    };
  }, [invalidate, queryClient]);

  const action = useMutation({
    mutationFn: async (input: ActionInput) => {
      switch (input.op) {
        case "start":
          return api.localRuntimeStart(input.slug);
        case "stop":
          return api.localRuntimeStop(input.slug);
        case "restart":
          return api.localRuntimeRestart(input.slug);
        case "delete":
          return api.localModelDelete(input.slug, input.model);
        case "load":
          return api.localModelLoad(input.slug, input.model);
        case "unload":
          return api.localModelUnload(input.slug, input.model);
      }
    },
    onSuccess: invalidate,
    onError: (err) => setError(errorMessage(err)),
  });

  const pull = useMutation({
    mutationFn: ({ slug, model }: { slug: string; model: string }) =>
      api.localModelPull(slug, model),
    onSuccess: () => {
      setError(null);
      invalidate();
    },
    onError: (err) => setError(errorMessage(err)),
  });

  // `useLocalRuntimes` polls `local_runtime_snapshot`, which only replays the
  // cached table. Refetching it here made the old "再検出" button a no-op; this
  // mutation runs a real sweep and installs its result.
  const refresh = useMutation({
    mutationFn: api.localRuntimeRefresh,
    onSuccess: (runtimes) => {
      setError(null);
      queryClient.setQueryData(LOCAL_RUNTIMES_QUERY_KEY, runtimes);
    },
    onError: (err) => setError(errorMessage(err)),
  });

  return (
    <Section>
      <Card title={t("ローカルランタイム管理")} icon={<ServerCog size={13} />}>
        <Row align="between">
          <p className="m-0 text-xs text-muted">
            {t(
              "検出済みのランタイムをアプリから起動・停止し、モデルの取得・ロード・削除を行えます。",
            )}
          </p>
          <Button
            size="sm"
            disabled={refresh.isPending}
            onClick={() => {
              setError(null);
              refresh.mutate();
            }}
          >
            {refresh.isPending ? (
              <Spinner size={12} />
            ) : (
              <RefreshCw size={12} />
            )}
            {t("再検出")}
          </Button>
        </Row>
        {error && <ErrorBox error={error} />}
        {runtimes.isError && <ErrorBox error={runtimes.error} />}
        {runtimes.isPending ? (
          <Spinner />
        ) : (runtimes.data?.length ?? 0) === 0 ? (
          <p className="m-0 text-xs text-muted">
            {t(
              "ランタイムがまだ検出されていません。「再検出」を実行してください。",
            )}
          </p>
        ) : (
          <div className="flex flex-col gap-3">
            {runtimes.data?.map((runtime) => (
              <RuntimeCard
                key={runtime.slug}
                runtime={runtime}
                jobs={(jobs.data ?? []).filter(
                  (job) => job.runtime === runtime.slug,
                )}
                pending={
                  (action.isPending &&
                    action.variables?.slug === runtime.slug) ||
                  (pull.isPending && pull.variables?.slug === runtime.slug)
                }
                onAction={(input) => action.mutate(input)}
                onPull={(model) => pull.mutate({ slug: runtime.slug, model })}
                onError={setError}
              />
            ))}
          </div>
        )}
      </Card>
    </Section>
  );
}

function RuntimeCard({
  runtime,
  jobs,
  pending,
  onAction,
  onPull,
  onError,
}: {
  runtime: LocalRuntimeSnapshotView;
  jobs: LocalJobView[];
  pending: boolean;
  onAction: (input: ActionInput) => void;
  onPull: (model: string) => void;
  onError: (message: string | null) => void;
}) {
  const [model, setModel] = useState("");
  const [showLogs, setShowLogs] = useState(false);
  // Logs used to ride along in every 10-second snapshot (up to 500 lines per
  // runtime). Fetch them only while the panel is open.
  const logs = useQuery({
    queryKey: ["local_runtime_logs", runtime.slug],
    queryFn: () => api.localRuntimeLogs(runtime.slug),
    enabled: showLogs,
    refetchInterval: showLogs ? 3000 : false,
  });
  const busy = ["starting", "stopping"].includes(runtime.state);
  const activeJob = jobs.find(
    (job) => job.status === "running" || job.status === "cancelling",
  );

  return (
    <div className="rounded-xl border border-line bg-surface p-3">
      <Row align="between">
        <div className="flex min-w-0 flex-wrap items-center gap-1.5">
          <span className="truncate text-sm font-medium text-fg">
            {runtime.label}
          </span>
          <Badge tone={stateTone(runtime.state)} size="sm">
            {stateLabel(runtime.state)}
          </Badge>
          {runtime.managed && <Badge size="sm">{t("アプリ起動")}</Badge>}
          {runtime.version && (
            <span className="font-mono text-2xs text-faint">
              v{runtime.version}
            </span>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {runtime.can_start && !runtime.online && (
            <Button
              size="sm"
              disabled={pending || busy}
              onClick={() => onAction({ op: "start", slug: runtime.slug })}
            >
              <Play size={12} /> {t("起動")}
            </Button>
          )}
          {runtime.online && runtime.can_stop && (
            <>
              <Button
                size="sm"
                disabled={pending || busy}
                onClick={() => onAction({ op: "restart", slug: runtime.slug })}
              >
                <RotateCw size={12} /> {t("再起動")}
              </Button>
              <Button
                size="sm"
                variant="ghost-danger"
                disabled={pending || busy}
                onClick={() => onAction({ op: "stop", slug: runtime.slug })}
              >
                <Square size={12} /> {t("停止")}
              </Button>
            </>
          )}
          {runtime.can_open_app && runtime.install_hint && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                onError(null);
                void api
                  .localRuntimeOpenApp(runtime.slug)
                  .catch((err) => onError(errorMessage(err)));
              }}
            >
              <ExternalLink size={12} /> {t("アプリを開く")}
            </Button>
          )}
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setShowLogs((value) => !value)}
            aria-pressed={showLogs}
          >
            <Terminal size={12} /> {t("ログ")}
          </Button>
        </div>
      </Row>

      <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-2xs text-faint">
        <span className="font-mono">{runtime.endpoint}</span>
        {runtime.install_hint && (
          <span className="truncate font-mono" title={runtime.install_hint}>
            {runtime.install_hint}
          </span>
        )}
        {runtime.executable && (
          <span className="truncate font-mono" title={runtime.executable}>
            {runtime.executable}
          </span>
        )}
        {runtime.launcher && <code>{runtime.launcher}</code>}
      </div>

      {runtime.error && <ErrorBox error={runtime.error} />}

      {runtime.can_pull && (
        <Row className="mt-3">
          <div className="min-w-48 flex-1">
            <Input
              placeholder={t("モデル名（例: qwen3:8b）")}
              value={model}
              onChange={(event) => setModel(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && model.trim()) {
                  onPull(model.trim());
                  setModel("");
                }
              }}
            />
          </div>
          <Button
            size="sm"
            disabled={
              !runtime.online || !model.trim() || pending || Boolean(activeJob)
            }
            onClick={() => {
              onPull(model.trim());
              setModel("");
            }}
          >
            <ArrowDownToLine size={12} /> {t("取得")}
          </Button>
        </Row>
      )}

      {activeJob && (
        <div className="mt-2 flex flex-col gap-1">
          <div className="flex items-center gap-2 text-xs text-muted">
            <Spinner size={12} />
            <span className="font-mono">{activeJob.model}</span>
            <Badge tone={jobStatusTone(activeJob.status)} size="sm">
              {jobStatusLabel(activeJob.status)}
            </Badge>
            {activeJob.message && (
              <span className="truncate text-2xs text-faint">
                {activeJob.message}
              </span>
            )}
            {activeJob.total > 0 && (
              <span className="font-mono text-2xs text-faint">
                {formatBytes(activeJob.completed)} /{" "}
                {formatBytes(activeJob.total)}
              </span>
            )}
            <span className="flex-1" />
            <Button
              size="sm"
              variant="ghost"
              disabled={activeJob.status === "cancelling"}
              onClick={() => {
                void api
                  .localJobCancel(activeJob.id)
                  .then((cancelled) => {
                    if (!cancelled) {
                      onError(t("このジョブはすでに終了しています"));
                    }
                  })
                  .catch((err) => onError(errorMessage(err)));
              }}
            >
              {t("キャンセル")}
            </Button>
          </div>
          {activeJob.total > 0 && (
            <div className="h-1.5 overflow-hidden rounded-full bg-inset">
              <div
                className="h-full bg-brand transition-[width]"
                style={{
                  width: `${Math.min(100, (activeJob.completed / activeJob.total) * 100)}%`,
                }}
              />
            </div>
          )}
        </div>
      )}

      {jobs
        .filter((job) => job.status !== "running")
        .slice(-3)
        .map((job) => (
          <div key={job.id} className="mt-1 text-xs text-muted">
            <Badge tone={jobStatusTone(job.status)} size="sm">
              {jobStatusLabel(job.status)}
            </Badge>{" "}
            <span className="font-mono">{job.model}</span>{" "}
            {job.error && (
              <span className="text-danger">{errorMessage(job.error)}</span>
            )}
          </div>
        ))}

      {runtime.models.length > 0 ? (
        <div className="mt-3 overflow-hidden rounded-lg border border-line">
          {runtime.models.map((item) => (
            <div
              key={item.id}
              className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line px-2.5 py-2 text-xs last:border-b-0"
            >
              <span className="min-w-0 flex-1 truncate font-mono text-fg">
                {item.id}
              </span>
              {item.loaded && (
                <Badge tone="success" size="sm">
                  {t("ロード中")}
                </Badge>
              )}
              {item.family && <Badge size="sm">{item.family}</Badge>}
              {item.parameter_size && (
                <Badge size="sm">{item.parameter_size}</Badge>
              )}
              {item.quantization && (
                <Badge size="sm">{item.quantization}</Badge>
              )}
              {(item.size_bytes ?? 0) > 0 && (
                <span className="text-2xs text-faint">
                  {formatBytes(item.size_bytes ?? 0)}
                </span>
              )}
              <span className="font-mono text-2xs text-faint">
                {formatContext(item.context_window)}
              </span>
              <div className="flex items-center gap-1">
                {runtime.can_load && !item.loaded && (
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    title={t("ロード")}
                    disabled={pending || !runtime.online}
                    onClick={() =>
                      onAction({
                        op: "load",
                        slug: runtime.slug,
                        model: item.id,
                      })
                    }
                  >
                    <ArrowUpFromLine size={12} />
                  </Button>
                )}
                {runtime.can_unload && item.loaded && (
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    title={t("アンロード")}
                    disabled={pending || !runtime.online}
                    onClick={() =>
                      onAction({
                        op: "unload",
                        slug: runtime.slug,
                        model: item.id,
                      })
                    }
                  >
                    <Power size={12} />
                  </Button>
                )}
                {runtime.can_delete && (
                  <Button
                    size="icon-sm"
                    variant="ghost-danger"
                    title={t("削除")}
                    disabled={pending || !runtime.online}
                    onClick={() => {
                      if (
                        window.confirm(
                          tf("「{model}」を削除しますか？", { model: item.id }),
                        )
                      ) {
                        onAction({
                          op: "delete",
                          slug: runtime.slug,
                          model: item.id,
                        });
                      }
                    }}
                  >
                    <Trash2 size={12} />
                  </Button>
                )}
              </div>
            </div>
          ))}
        </div>
      ) : (
        <p className="mt-3 text-xs text-muted">
          {t("モデルがありません。上の入力欄から取得できます。")}
        </p>
      )}

      {showLogs && (
        <div className="mt-3">
          <Field label={t("直近のログ")}>
            {logs.isPending ? (
              <Spinner />
            ) : logs.isError ? (
              <ErrorBox error={logs.error} />
            ) : (
              <pre className="m-0 max-h-56 overflow-auto rounded-lg bg-inset p-2 font-mono text-2xs text-muted">
                {(logs.data ?? []).length > 0
                  ? (logs.data ?? []).join("\n")
                  : t("ログはまだありません")}
              </pre>
            )}
          </Field>
        </div>
      )}
    </div>
  );
}
