import { useMemo } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle2, ChevronDown, ChevronRight, CircleAlert, ClipboardList, Clock3, FileText,
  Globe2, History, ListChecks, Loader2, Pencil, RotateCcw, Search, ShieldAlert, Terminal, Wrench,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { formatDuration, shortId } from "@/lib/format";
import {
  describeToolCall, isFileWrite, reduceRunEvents, type TodoItem, type ToolActivity,
} from "@/lib/runEvents";
import { useRunEvents } from "@/lib/runStore";
import { Card, PanelHeader } from "@/components/layout";
import { useUi } from "@/lib/ui";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/alert";
import { Spinner } from "@/components/ui/spinner";

function toolIcon(name: string) {
  if (name === "files") return FileText;
  if (name === "edit") return Pencil;
  if (name === "terminal") return Terminal;
  if (name === "web_search") return Search;
  if (name === "network") return Globe2;
  if (name === "todo") return ListChecks;
  if (name === "user_plan") return ClipboardList;
  return Wrench;
}

function statusOf(tool: ToolActivity): {
  label: string;
  Icon: typeof CheckCircle2;
  spin: boolean;
  tone: NonNullable<BadgeProps["tone"]>;
} {
  if (tool.status === "running") {
    return { label: t("実行中"), Icon: Loader2, spin: true, tone: "warning" };
  }
  if (tool.status === "error") {
    return { label: t("失敗"), Icon: CircleAlert, spin: false, tone: "danger" };
  }
  return { label: t("成功"), Icon: CheckCircle2, spin: false, tone: "success" };
}

function ToolRow({ tool }: { tool: ToolActivity }) {
  const openDiff = useUi((s) => s.openDiff);
  const Icon = toolIcon(tool.name);
  const status = statusOf(tool);
  const StatusIcon = status.Icon;
  const args = tool.arguments as Record<string, unknown> | null;
  const elapsed =
    tool.startedAt && tool.finishedAt
      ? new Date(tool.finishedAt).getTime() - new Date(tool.startedAt).getTime()
      : null;

  // `files.write` carries the whole new content. `edit` does not: its
  // `result.preview` is a `@@ line N @@` region, and falling back to
  // `String(args.content)` produced the literal string "undefined" — either way
  // the diff was nonsense. Hand the anchor and its replacement over instead and
  // let the backend apply them to the file on disk.
  const a = (args ?? {}) as Record<string, unknown>;
  const writeInfo: { path: string; content: string; oldString?: string; newString?: string } | null =
    isFileWrite(tool)
      ? tool.name === "edit"
        ? {
            path: String(a.path),
            content: "",
            oldString: typeof a.old_string === "string" ? a.old_string : "",
            newString: typeof a.new_string === "string" ? a.new_string : "",
          }
        : { path: String(a.path), content: String(a.content) }
      : null;

  // One call for both the tooltip and the body: it formats every argument of
  // the call, and it was being run twice per row on every render.
  const description = describeToolCall(tool);

  return (
    <div className="flex gap-2 px-3 py-2.5">
      <Icon size={14} className="mt-0.5 shrink-0 text-faint" />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-center justify-between gap-2">
          <span className="truncate font-mono text-xs font-medium text-fg" title={description}>
            {tool.name}
          </span>
          <Badge tone={status.tone} size="sm" className="shrink-0">
            <StatusIcon size={10} className={status.spin ? "animate-spin" : undefined} />
            {status.label}
          </Badge>
        </div>
        <span className="break-all text-xs text-muted">{description}</span>
        {elapsed != null && (
          <span className="text-xs text-faint">{formatDuration(elapsed)}</span>
        )}
        {writeInfo && (
          <button
            type="button"
            onClick={() =>
              openDiff({ path: writeInfo.path, content: writeInfo.content, label: writeInfo.path })
            }
            className={cn(
              "mt-0.5 inline-flex w-fit max-w-full items-center gap-1 rounded-md border border-line",
              "bg-inset px-1.5 py-0.5 text-xs text-muted",
              "transition-colors hover:border-brand hover:text-fg",
            )}
          >
            <FileText size={11} className="shrink-0" />
            <span className="truncate">{writeInfo.path}</span>
            <ChevronDown size={11} className="shrink-0" />
          </button>
        )}
        {tool.arguments != null && (
          <details className="group">
            <summary className="flex cursor-pointer list-none items-center gap-1 text-xs text-faint transition-colors hover:text-muted">
              <ChevronRight
                size={11}
                aria-hidden="true"
                className="shrink-0 transition-transform group-open:rotate-90"
              />
              {t("引数")}
            </summary>
            <pre className="mt-1 max-h-52 overflow-auto rounded-md bg-inset p-2 font-mono text-xs leading-4 text-muted">
              {safeJson(tool.arguments)}
            </pre>
          </details>
        )}
        {tool.error && (
          <span className="break-words text-xs text-danger">{tool.error}</span>
        )}
        {tool.result != null && (
          <details className="group">
            <summary className="flex cursor-pointer list-none items-center gap-1 text-xs text-faint transition-colors hover:text-muted">
              <ChevronRight
                size={11}
                aria-hidden="true"
                className="shrink-0 transition-transform group-open:rotate-90"
              />
              {t("結果")}
            </summary>
            <pre className="mt-1 max-h-52 overflow-auto rounded-md bg-inset p-2 font-mono text-xs leading-4 text-muted">
              {safeJson(tool.result)}
            </pre>
          </details>
        )}
      </div>
    </div>
  );
}

/** The agent's current plan, so the user can see what it thinks it is doing. */
function TodoList({ items }: { items: TodoItem[] }) {
  const done = items.filter((i) => i.status === "completed").length;
  return (
    <Card
      title={t("計画")}
      icon={<ListChecks size={13} />}
      action={
        <span className="font-mono text-xs font-normal text-faint">
          {done}/{items.length}
        </span>
      }
      bodyClassName="gap-0 p-2.5"
    >
      <ul className="flex flex-col gap-1.5">
        {items.map((item) => (
          <li
            key={item.id}
            className={cn(
              "flex items-start gap-2 text-xs leading-5",
              item.status === "completed" ? "text-faint line-through" : "text-fg",
            )}
          >
            <span className="mt-0.5 shrink-0">
              {item.status === "completed" ? (
                <CheckCircle2 size={12} className="text-success" />
              ) : item.status === "in_progress" ? (
                <Spinner size={12} className="text-warning" />
              ) : (
                <span className="block size-3 rounded-sm border border-line" />
              )}
            </span>
            <span className="min-w-0 flex-1">{item.content}</span>
          </li>
        ))}
      </ul>
    </Card>
  );
}

function safeJson(value: unknown): string {
  const text = JSON.stringify(value, null, 2) ?? "";
  return text.length > 4000 ? `${text.slice(0, 4000)}…` : text;
}

/** Right-panel activity view: live run or a historical run's replayed events. */
export function ActivityPanel() {
  const liveRunId = useRunEvents((s) => s.liveRunId);
  const liveTimeline = useRunEvents((s) => s.timeline);
  const activityRunId = useUi((s) => s.activityRunId);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const setActivityRunId = useUi((s) => s.setActivityRunId);

  const runId = activityRunId ?? liveRunId;
  const isLive = runId != null && runId === liveRunId;
  const startRun = useRunEvents((s) => s.startRun);
  const activeConversationId = useRunEvents((s) => s.conversationId);
  const queryClient = useQueryClient();

  const history = useQuery({
    queryKey: ["run_events", runId],
    queryFn: () => api.listRunEvents(runId!),
    enabled: !!runId && !isLive,
    staleTime: Infinity,
  });

  const checkpoints = useQuery({
    queryKey: ["checkpoints", runId],
    queryFn: () => api.listCheckpoints(runId!),
    enabled: !!runId && !isLive,
    staleTime: Infinity,
  });

  // Replaying the persisted log is O(events), and this component also
  // subscribes to the live stream — so doing it in the render body meant that
  // viewing one run while another streamed re-folded the whole history on
  // every 60ms flush.
  const replayed = useMemo(
    () => (history.data ? reduceRunEvents(runId!, history.data) : null),
    [history.data, runId],
  );
  const timeline = isLive ? liveTimeline : replayed;
  const empty = !runId || (timeline && timeline.tools.length === 0);

  return (
    <>
      <PanelHeader
        icon={<Wrench size={14} />}
        title={t("このRunのアクティビティ")}
        titleClass="text-sm font-medium"
        trailing={runId ? <Badge size="sm">{shortId(runId)}</Badge> : null}
        onClose={() => {
          setActivityRunId(null);
          setRightPanel("none");
        }}
      />

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
        {empty && (
          <p className="px-2 py-6 text-center text-xs text-faint">{t("アクティビティはありません")}</p>
        )}
        {runId && history.isError && (
          <p role="alert" className="px-2 py-6 text-center text-xs text-danger">
            {errorMessage(history.error) || t("エラー")}
          </p>
        )}
        {runId && !history.isError && !timeline && <Skeleton className="h-16 w-full" />}

        {runId && timeline && timeline.tools.length > 0 && (
          <div className="divide-y divide-line overflow-hidden rounded-lg border border-line">
            {timeline.tools.map((tool) => (
              <ToolRow key={tool.id} tool={tool} />
            ))}
          </div>
        )}

        {runId && timeline && timeline.todos.length > 0 && (
          <TodoList items={timeline.todos} />
        )}

        {runId && timeline?.verificationFailed && (
          <Card
            className="border-danger"
            title={t("検証に失敗しました")}
            icon={<ShieldAlert size={13} className="text-danger" />}
            bodyClassName="p-2.5"
          >
            <p className="m-0 text-xs text-muted">{timeline.verificationFailed}</p>
          </Card>
        )}

        {runId && timeline?.verification && !timeline.verificationFailed && (
          <Card
            title={t("変更の確認中")}
            icon={<ShieldAlert size={13} />}
            bodyClassName="p-2.5"
          >
            <p className="m-0 text-xs text-muted">
              {timeline.verification.reason || timeline.verification.tool}
            </p>
          </Card>
        )}

        {runId && !isLive && (checkpoints.data?.length ?? 0) > 0 && (
          <Card
            title={t("チェックポイント")}
            icon={<History size={13} />}
            bodyClassName="gap-0 p-0"
          >
            <ul className="flex flex-col divide-y divide-line">
              {(checkpoints.data ?? []).map((c) => (
                <li key={c.id} className="flex items-center justify-between gap-2 px-2.5 py-1.5">
                  <span className="flex items-center gap-1.5 text-xs text-muted">
                    <Clock3 size={11} /> {t("ステップ")} {c.step_index}
                  </span>
                  <Button
                    size="sm"
                    onClick={() => {
                      void api
                        .resumeRun({ id: runId, from_checkpoint: c.id, unknown_actions: [] })
                        .then(() => {
                          // Track the resumed run. Without this every event it
                          // emitted was dropped (`run_id !== liveRunId`) and the
                          // history query — `staleTime: Infinity` — was never
                          // refetched, so the panel froze on the pre-resume
                          // snapshot.
                          startRun(runId, activeConversationId);
                          void queryClient.invalidateQueries({
                            queryKey: ["run_events", runId],
                          });
                          void queryClient.invalidateQueries({
                            queryKey: ["checkpoints", runId],
                          });
                        });
                    }}
                  >
                    <RotateCcw size={11} /> {t("再開")}
                  </Button>
                </li>
              ))}
            </ul>
          </Card>
        )}
      </div>
    </>
  );
}
