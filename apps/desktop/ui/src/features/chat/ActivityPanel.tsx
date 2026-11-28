import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  ClipboardList,
  Code2,
  Clock3,
  Download,
  FileText,
  Globe2,
  History,
  ListChecks,
  Pencil,
  RotateCcw,
  Search,
  ShieldAlert,
  Terminal,
  Undo2,
  Upload,
  Wrench,
} from "lucide-react";
import { api } from "@/lib/api";
import type { RunEventPayload } from "@/lib/api";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { formatDuration, shortId } from "@/lib/format";
import {
  describeToolCall,
  isFileWrite,
  optString,
  record,
  buildTrajectory,
  reduceRunEvents,
  type TodoItem,
  type ToolActivity,
  type TrajectoryPoint,
} from "@/lib/runEvents";
import { useRunEvents } from "@/lib/runStore";
import { Card, PanelHeader, PANEL_BODY } from "@/components/layout";
import { useUi } from "@/lib/ui";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";
import {
  EmptyState,
  ErrorBadge,
  ErrorBox,
  ListSkeleton,
  payloadBlock,
} from "@/components/shared";
import { TerminalResultView } from "./TerminalResultView";
import { ContextCompactionNotice } from "./ContextCompaction";

function toolIcon(name: string) {
  if (name === "files") return FileText;
  if (name === "edit") return Pencil;
  if (name === "terminal") return Terminal;
  if (name === "web_search") return Search;
  if (name === "network") return Globe2;
  if (name === "todo") return ListChecks;
  if (name === "user_plan") return ClipboardList;
  if (name === "lsp") return Code2;
  return Wrench;
}

function statusOf(tool: ToolActivity): {
  label: string;
  Icon: typeof CheckCircle2;
  spin: boolean;
  tone: NonNullable<BadgeProps["tone"]>;
} {
  if (tool.status === "running") {
    return {
      label: t("実行中"),
      Icon: CheckCircle2,
      spin: true,
      tone: "warning",
    };
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
  // Shared `record`/`optString` guards (see `lib/runEvents.ts`): the ad-hoc
  // `as Record<…>` cast plus per-field `typeof` checks used to live here.
  const args = record(tool.arguments);
  const elapsed =
    tool.startedAt && tool.finishedAt
      ? new Date(tool.finishedAt).getTime() - new Date(tool.startedAt).getTime()
      : null;

  // `files.write` carries the whole new content. `edit` does not: its
  // `result.preview` is a `@@ line N @@` region, and falling back to
  // `String(args.content)` produced the literal string "undefined" — either way
  // the diff was nonsense. Hand the anchor and its replacement over instead and
  // let the backend apply them to the file on disk. A non-string path hides
  // the button entirely instead of opening a diff for `"undefined"`.
  const rawPath = optString(args.path);
  const writeInfo: {
    path: string;
    content: string;
    oldString?: string;
    newString?: string;
  } | null =
    isFileWrite(tool) && rawPath
      ? tool.name === "edit"
        ? {
            path: rawPath,
            content: "",
            oldString: optString(args.old_string) ?? "",
            newString: optString(args.new_string) ?? "",
          }
        : (() => {
            const content = optString(args.content);
            return content !== undefined ? { path: rawPath, content } : null;
          })()
      : null;

  // One call for both the tooltip and the body: it formats every argument of
  // the call, and it was being run twice per row on every render.
  const description = describeToolCall(tool);

  return (
    <div className="flex gap-2 px-3 py-2.5">
      <Icon size={13} className="mt-0.5 shrink-0 text-faint" />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex items-center justify-between gap-2">
          <span
            className="truncate font-mono text-xs font-medium text-fg"
            title={description}
          >
            {tool.name}
          </span>
          <Badge tone={status.tone} size="sm" className="shrink-0">
            {status.spin ? (
              <Spinner size={10} className="text-inherit" />
            ) : (
              <StatusIcon size={10} />
            )}
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
              openDiff({
                path: writeInfo.path,
                content: writeInfo.content,
                oldString: writeInfo.oldString,
                newString: writeInfo.newString,
                label: writeInfo.path,
              })
            }
            className={cn(
              "mt-0.5 inline-flex w-fit max-w-full items-center gap-1 rounded-md border border-line",
              "bg-inset px-1.5 py-0.5 text-xs text-muted",
              "transition-colors hover:border-brand hover:text-fg",
            )}
          >
            <FileText size={12} className="shrink-0" />
            <span className="truncate">{writeInfo.path}</span>
            <ChevronDown size={12} className="shrink-0" />
          </button>
        )}
        {tool.arguments != null && (
          <details className="group">
            <summary className="flex cursor-pointer list-none items-center gap-1 text-xs text-faint transition-colors hover:text-muted">
              <ChevronRight
                size={12}
                aria-hidden="true"
                className="shrink-0 transition-transform group-open:rotate-90"
              />
              {t("引数")}
            </summary>
            <pre className={cn("mt-1 max-h-52", payloadBlock)}>
              {safeJson(tool.arguments)}
            </pre>
          </details>
        )}
        {tool.error && <ErrorBadge error={tool.error} />}
        {tool.result != null && (
          <details className="group" open={tool.name === "terminal"}>
            <summary className="flex cursor-pointer list-none items-center gap-1 text-xs text-faint transition-colors hover:text-muted">
              <ChevronRight
                size={12}
                aria-hidden="true"
                className="shrink-0 transition-transform group-open:rotate-90"
              />
              {t("結果")}
            </summary>
            <div className="mt-1">
              {tool.name === "terminal" && typeof tool.result === "object" ? (
                <TerminalResultView
                  result={tool.result as Record<string, unknown>}
                />
              ) : (
                <pre className={cn("max-h-52", payloadBlock)}>
                  {safeJson(tool.result)}
                </pre>
              )}
            </div>
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
              item.status === "completed"
                ? "text-faint line-through"
                : "text-fg",
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

/** Stable empty fallback so the trajectory memo does not rebuild. */
const EMPTY_EVENTS: RunEventPayload[] = [];

/** Chronological event list backing the Trajectory tab: every event in
 *  sequence order with a one-line summary, filterable by kind and text. */
function TrajectoryView({ points }: { points: TrajectoryPoint[] }) {
  const [kind, setKind] = useState("all");
  const [query, setQuery] = useState("");
  const kinds = useMemo(
    () => [...new Set(points.map((p) => p.kind))].sort(),
    [points],
  );
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return points.filter(
      (p) =>
        (kind === "all" || p.kind === kind) &&
        (q === "" ||
          p.label.toLowerCase().includes(q) ||
          p.detail.toLowerCase().includes(q) ||
          p.kind.toLowerCase().includes(q)),
    );
  }, [points, kind, query]);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex gap-1.5">
        <div className="relative min-w-0 flex-1">
          <Search
            size={12}
            aria-hidden="true"
            className="pointer-events-none absolute top-1/2 left-2 -translate-y-1/2 text-faint"
          />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("イベントを検索")}
            className="w-full rounded-md border border-line bg-inset py-1 pr-2 pl-7 text-xs text-fg placeholder:text-faint"
          />
        </div>
        <select
          value={kind}
          onChange={(e) => setKind(e.target.value)}
          aria-label={t("種類で絞り込み")}
          className="max-w-36 rounded-md border border-line bg-inset px-1.5 py-1 text-xs text-fg"
        >
          <option value="all">{t("すべて")}</option>
          {kinds.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
      </div>
      {filtered.length === 0 ? (
        <EmptyState compact title={t("イベントはありません")} />
      ) : (
        <ol className="flex max-h-96 flex-col gap-px overflow-y-auto rounded-lg border border-line bg-surface p-1 shadow-xs">
          {filtered.map((p) => (
            <li
              key={p.sequence}
              className="flex items-baseline gap-2 rounded px-1.5 py-1 text-xs"
            >
              <span className="shrink-0 font-mono text-2xs text-faint">
                #{p.sequence}
              </span>
              <span className="shrink-0 font-mono text-2xs text-faint">
                {new Date(p.createdAt).toLocaleTimeString()}
              </span>
              <span
                className="min-w-0 flex-1 truncate"
                title={p.detail || p.kind}
              >
                <span className="font-mono font-medium text-fg">{p.label}</span>
                {p.detail && <span className="text-muted"> — {p.detail}</span>}
              </span>
            </li>
          ))}
        </ol>
      )}
      <p className="m-0 text-xs text-faint">
        {tf("{count} 件のイベント", { count: filtered.length })}
      </p>
    </div>
  );
}

/** Export (JSONL/Markdown) and import (JSONL) for one run's event log. */
function RunIoCard({ runId }: { runId: string }) {
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState<"jsonl" | "md" | "import" | null>(null);
  const queryClient = useQueryClient();

  const save = async (format: "jsonl" | "md") => {
    setBusy(format);
    setNote(null);
    try {
      const content =
        format === "jsonl"
          ? await api.exportRunEventsJsonl(runId)
          : await api.exportRunMarkdown(runId);
      const ext = format === "jsonl" ? "jsonl" : "md";
      const path = await api.pickSaveFile({
        title: t("Runのエクスポート"),
        defaultName: `run-${shortId(runId)}.${ext}`,
        filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
      });
      if (!path) return;
      await api.saveTextFile(path, content);
      setNote(t("エクスポートしました。"));
    } catch (err) {
      setNote(errorMessage(err) || t("エラー"));
    } finally {
      setBusy(null);
    }
  };

  const load = async () => {
    setBusy("import");
    setNote(null);
    try {
      const paths = await api.pickOpenFile({
        title: t("Runのインポート"),
        filters: [{ name: "JSONL", extensions: ["jsonl"] }],
      });
      const path = paths?.[0];
      if (!path) return;
      const count = await api.importRunEventsFile(runId, path);
      setNote(tf("{count} 件のイベントを取り込みました。", { count }));
      void queryClient.invalidateQueries({ queryKey: ["run_events", runId] });
    } catch (err) {
      setNote(errorMessage(err) || t("エラー"));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Card
      title={t("エクスポート / インポート")}
      icon={<Download size={13} />}
      bodyClassName="p-2.5"
    >
      <div className="flex flex-wrap gap-1.5">
        <Button
          size="sm"
          disabled={busy !== null}
          onClick={() => void save("jsonl")}
        >
          <Download size={12} /> JSONL
        </Button>
        <Button
          size="sm"
          disabled={busy !== null}
          onClick={() => void save("md")}
        >
          <Download size={12} /> Markdown
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={busy !== null}
          onClick={() => void load()}
        >
          <Upload size={12} /> {t("取り込み")}
        </Button>
      </div>
      {note && <p className="m-0 text-xs text-muted">{note}</p>}
    </Card>
  );
}

function safeJson(value: unknown): string {
  const text = JSON.stringify(value, null, 2) ?? "";
  return text.length > 4000 ? `${text.slice(0, 4000)}…` : text;
}

/** Right-panel activity view: live run or a historical run's replayed events. */
export function ActivityPanel({
  embedded = false,
}: { embedded?: boolean } = {}) {
  const conversationId = useUi((s) => s.conversationId);
  const liveEntry = useRunEvents((s) =>
    conversationId ? s.byConversation[conversationId] : undefined,
  );
  const liveRunId = liveEntry?.runId ?? null;
  const liveTimeline = liveEntry?.timeline ?? null;
  const activityRunId = useUi((s) => s.activityRunId);
  const closeRightPanel = useUi((s) => s.closeRightPanel);
  const setActivityRunId = useUi((s) => s.setActivityRunId);

  const runId = activityRunId ?? liveRunId;
  const isLive = runId != null && runId === liveRunId;
  const startRun = useRunEvents((s) => s.startRun);
  const activeConversationId = conversationId;
  const queryClient = useQueryClient();

  const history = useQuery({
    queryKey: ["run_events", runId],
    queryFn: () => api.listRunEvents(runId!),
    enabled: !!runId && !isLive,
    staleTime: Infinity,
  });

  const [revertNote, setRevertNote] = useState<string | null>(null);
  const { confirm, dialog } = useConfirm();
  const workspaceChanges = useQuery({
    queryKey: ["run_workspace_changes", runId],
    queryFn: () => api.runWorkspaceChanges(runId!),
    enabled: !!runId && !isLive,
    staleTime: 5_000,
  });
  const revert = useMutation({
    mutationFn: (id: string) => api.revertRunWorkspace(id),
    onSuccess: (count) => {
      setRevertNote(
        tf("{count} 件のファイルを実行前の内容に戻しました。", { count }),
      );
      void queryClient.invalidateQueries({ queryKey: ["file_diff"] });
      void queryClient.invalidateQueries({ queryKey: ["git_status"] });
      void queryClient.invalidateQueries({
        queryKey: ["run_workspace_changes", runId],
      });
      void queryClient.invalidateQueries({ queryKey: ["checkpoints", runId] });
    },
    onError: (error) => setRevertNote(errorMessage(error) || t("エラー")),
  });
  const revertPaths = useMutation({
    mutationFn: (paths: string[]) => api.revertRunWorkspacePaths(runId!, paths),
    onSuccess: (count) => {
      setRevertNote(
        tf("{count} 件のファイルを実行前の内容に戻しました。", { count }),
      );
      void queryClient.invalidateQueries({ queryKey: ["file_diff"] });
      void queryClient.invalidateQueries({ queryKey: ["git_status"] });
      void queryClient.invalidateQueries({
        queryKey: ["run_workspace_changes", runId],
      });
    },
    onError: (error) => setRevertNote(errorMessage(error) || t("エラー")),
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

  // Trajectory tab: raw chronology for the timeline view. Live runs read the
  // bounded buffer the store keeps per run; finished runs read the persisted
  // history query both tabs already share.
  const [tab, setTab] = useState<"activity" | "trajectory">("activity");
  const liveEvents = liveEntry?.events ?? EMPTY_EVENTS;
  const rawEvents = isLive ? liveEvents : (history.data ?? EMPTY_EVENTS);
  const points = useMemo(
    () => (runId ? buildTrajectory(rawEvents) : []),
    [runId, rawEvents],
  );

  // Resuming a checkpoint used to reject unhandled: the button never disabled
  // and the panel silently stayed on the pre-resume snapshot.
  const [resumingId, setResumingId] = useState<string | null>(null);
  const [resumeError, setResumeError] = useState<string | null>(null);
  const resumeFromCheckpoint = async (id: string, checkpointId: string) => {
    setResumingId(checkpointId);
    setResumeError(null);
    try {
      await api.resumeRun({
        id,
        from_checkpoint: checkpointId,
        unknown_actions: [],
      });
      // Track the resumed run. Without this every event it emitted was dropped
      // (`run_id !== liveRunId`) and the history query — `staleTime: Infinity`
      // — was never refetched, so the panel froze on the pre-resume snapshot.
      startRun(id, activeConversationId);
      void queryClient.invalidateQueries({ queryKey: ["run_events", id] });
      void queryClient.invalidateQueries({ queryKey: ["checkpoints", id] });
    } catch (err) {
      setResumeError(errorMessage(err) || t("エラー"));
    } finally {
      setResumingId(null);
    }
  };

  return (
    <>
      {!embedded && (
        <PanelHeader
          icon={<Wrench size={14} />}
          title={t("このRunのアクティビティ")}
          trailing={runId ? <Badge size="sm">{shortId(runId)}</Badge> : null}
          onClose={() => {
            setActivityRunId(null);
            closeRightPanel();
          }}
        />
      )}

      {/* `[&>*]:shrink-0` (part of `PANEL_BODY`) is load-bearing. This column has
          a definite height (it scrolls), so its children are flex items that
          shrink to fit it — and a child with `overflow: hidden`, which every
          `Card` has, has no automatic minimum size to fall back on. The cards
          were silently squashed and cut off instead of the column scrolling. */}
      <div className={PANEL_BODY}>
        {runId && (
          <div
            className="flex gap-1 rounded-lg border border-line bg-inset p-0.5"
            role="tablist"
          >
            {(
              [
                ["activity", t("アクティビティ")],
                ["trajectory", t("時系列")],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                type="button"
                role="tab"
                aria-selected={tab === key}
                onClick={() => setTab(key)}
                className={cn(
                  "flex-1 rounded-md px-2 py-1 text-xs transition-colors",
                  tab === key
                    ? "bg-surface font-medium text-fg shadow-xs"
                    : "text-faint hover:text-muted",
                )}
              >
                {label}
              </button>
            ))}
          </div>
        )}
        {tab === "trajectory" ? (
          <>
            {runId && <TrajectoryView points={points} />}
            {runId && <RunIoCard runId={runId} />}
          </>
        ) : (
          <>
            {runId && !history.isError && !timeline && (
              <ListSkeleton rows={1} />
            )}
            {runId && history.isError && <ErrorBox error={history.error} />}
            {resumeError && <ErrorBox error={resumeError} />}
            {empty && (
              <EmptyState compact title={t("アクティビティはありません")} />
            )}

            {runId && timeline && timeline.tools.length > 0 && (
              <div className="divide-y divide-line overflow-hidden rounded-lg border border-line bg-surface shadow-xs">
                {timeline.tools.map((tool) => (
                  <ToolRow key={tool.id} tool={tool} />
                ))}
              </div>
            )}

            {runId && timeline && timeline.todos.length > 0 && (
              <TodoList items={timeline.todos} />
            )}

            {runId && timeline && timeline.contextCompactions.length > 0 && (
              <ContextCompactionNotice items={timeline.contextCompactions} />
            )}

            {runId && timeline?.verificationFailed && (
              <Card
                className="border-danger"
                title={t("検証に失敗しました")}
                icon={<ShieldAlert size={13} className="text-danger" />}
                bodyClassName="p-2.5"
              >
                <p className="m-0 text-xs text-muted">
                  {timeline.verificationFailed}
                </p>
              </Card>
            )}

            {runId &&
              timeline?.verification &&
              !timeline.verificationFailed && (
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
                    <li
                      key={c.id}
                      className="flex items-center justify-between gap-2 px-2.5 py-1.5"
                    >
                      <span className="flex items-center gap-1.5 text-xs text-muted">
                        <Clock3 size={13} /> {t("ステップ")} {c.step_index}
                      </span>
                      <Button
                        size="sm"
                        disabled={resumingId === c.id}
                        onClick={() => void resumeFromCheckpoint(runId, c.id)}
                      >
                        <RotateCcw size={12} /> {t("再開")}
                      </Button>
                    </li>
                  ))}
                </ul>
              </Card>
            )}

            {runId && !isLive && (
              <Card
                title={t("実行の取り消し")}
                icon={<Undo2 size={13} />}
                bodyClassName="p-2.5"
              >
                <p className="m-0 text-xs text-muted">
                  {workspaceChanges.data?.available
                    ? t(
                        "このRunの変更をGitのスナップショットまで戻します。edit / files に加え、ターミナルが作成・変更したファイルも対象です。Run終了後に手で加えた変更も失われます。",
                      )
                    : t(
                        "このRunが edit / files で変更したファイルを、実行前の内容に戻します。ターミナル経由の変更は対象外です。",
                      )}
                </p>
                {workspaceChanges.data?.available &&
                  workspaceChanges.data.files.length > 0 && (
                    <details className="group text-xs">
                      <summary className="cursor-pointer list-none text-faint transition-colors hover:text-muted">
                        {tf("{count} 件の変更を確認", {
                          count: workspaceChanges.data.files.length,
                        })}
                      </summary>
                      <ul className="mt-1 flex max-h-40 list-none flex-col gap-0.5 overflow-y-auto p-0 font-mono text-2xs text-muted">
                        {workspaceChanges.data.files.map((file) => (
                          <li
                            key={file.path}
                            className="flex items-center gap-1.5"
                          >
                            <span className="w-4 shrink-0 text-faint">
                              {file.status}
                            </span>
                            <span
                              className="min-w-0 flex-1 truncate"
                              title={file.path}
                            >
                              {file.path}
                            </span>
                            <Button
                              variant="ghost"
                              size="xs"
                              className="shrink-0 gap-1 px-1.5 text-2xs"
                              disabled={revertPaths.isPending}
                              title={tf("{path} を実行前に戻す", {
                                path: file.path,
                              })}
                              onClick={async () => {
                                if (
                                  await confirm({
                                    title:
                                      t("このファイルの変更を取り消しますか？"),
                                    description: t(
                                      "選択したファイルだけを実行前の内容に戻します。Run終了後にこのファイルへ手で加えた変更も失われます。",
                                    ),
                                    confirmLabel: t("戻す"),
                                    tone: "danger",
                                  })
                                ) {
                                  revertPaths.mutate([file.path]);
                                }
                              }}
                            >
                              <Undo2 size={11} aria-hidden="true" />
                              {t("戻す")}
                            </Button>
                          </li>
                        ))}
                      </ul>
                    </details>
                  )}
                <Button
                  variant="ghost-danger"
                  size="sm"
                  disabled={revert.isPending}
                  onClick={async () => {
                    if (
                      await confirm({
                        title: t("このRunの変更を取り消しますか？"),
                        description: t(
                          "実行前の内容に上書きします。取り消し後に手で加えた変更も失われます。",
                        ),
                        confirmLabel: t("取り消す"),
                        tone: "danger",
                      })
                    ) {
                      revert.mutate(runId);
                    }
                  }}
                >
                  <Undo2 size={12} /> {t("実行前の状態に戻す")}
                </Button>
                {revertNote && (
                  <p className="m-0 text-xs text-muted">{revertNote}</p>
                )}
                {dialog}
              </Card>
            )}
            {runId && <RunIoCard runId={runId} />}
          </>
        )}
      </div>
    </>
  );
}
