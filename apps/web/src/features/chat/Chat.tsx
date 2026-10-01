import { t } from "@/app/i18n";
import { api, type Row } from "@/app/api";
import { uuid } from "@/app/uuid";
import { ja } from "@/app/strings";
import type { components } from "@/generated/api";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowUp,
  Bot,
  Brain,
  FileText,
  FolderKanban,
  MessageSquare,
  Paperclip,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Square,
  ThumbsDown,
  ThumbsUp,
  UserCog,
  X,
} from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import Markdown from "react-markdown";
import { useNavigate, useParams } from "react-router-dom";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

import { ErrorBox, useRows } from "@/components/shared";
import { ModelPicker } from "@/components/model-picker";
import { SelectMenu } from "@/components/select-menu";
import { Toggle } from "@/components/toggle";
import { ModeStatus, ReasoningSummary } from "./ModeStatus";
import { RunBudget } from "./RunBudget";
import { RunActivity, type ToolCallHistory } from "./RunActivity";
import { ArtifactCard } from "./ArtifactCard";
import { BrowserPanel } from "./BrowserPanel";
import { TodoPane } from "./TodoPane";
import {
  BudgetExtensionPanel,
  CheckpointsPanel,
  StagnationPanel,
} from "./AgentLoop";
import { randomGreeting } from "./greeting";
import {
  liveToolCalls,
  parseRunEvent,
  type Artifact,
  type ChatMode,
  type ConversationHistory,
  type PermissionRule,
  type Run,
  type RunEvent,
  type SendMessage,
  type StagnationFinding,
  type Tool,
} from "./types";

type ChatModel = Row & { data: { capabilities?: Record<string, boolean | null>; overrides?: Record<string, boolean | null>; reasoning_control?: boolean; tool_probe?: { status?: string } } };
type ChatAgent = Row & { data: { mode: ChatMode; model_id?: string; name: string; tool_ids?: string[]; max_seconds?: number; max_steps?: number; max_tool_calls?: number } };
type Feedback = components["schemas"]["FeedbackInput"]["value"];
const modes = ["chat", "thinking", "agent"] as const satisfies readonly ChatMode[];

export function Chat() {
  const { id, projectId } = useParams();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const models = useRows<ChatModel>("/models"),
    projects = useRows<Row>("/projects"),
    agents = useRows<ChatAgent>("/agents"),
    tools = useQuery<Tool[]>({
      queryKey: ["/tools"],
      queryFn: () => api("/tools"),
    }),
    permissionRules = useRows<PermissionRule>("/permission-rules");
  const [model, setModel] = useState("auto"),
    [temporaryMode, setTemporaryMode] = useState(false),
    [allowTools, setAllowTools] = useState(false),
    [mode, setMode] = useState<ChatMode>("chat"),
    [researchMode, setResearchMode] = useState(false),
    [selectedProject, setSelectedProject] = useState(projectId || ""),
    [agent, setAgent] = useState(""),
    [text, setText] = useState(""),
    [error, setError] = useState<unknown>(null),
    [busy, setBusy] = useState(false),
    [attachments, setAttachments] = useState<Artifact[]>([]),
    [runId, setRunId] = useState(""),
    [streamText, setStreamText] = useState(""),
    [reasoning, setReasoning] = useState(""),
    [events, setEvents] = useState<RunEvent[]>([]),
    [streamConnected, setStreamConnected] = useState(false);
  const [welcome] = useState(() => randomGreeting());
  const fileRef = useRef<HTMLInputElement>(null),
    bottom = useRef<HTMLDivElement>(null);
  const eventSourceRef = useRef<EventSource | null>(null);
  const pendingSend = useRef<{
    signature: string;
    key: string;
    conversation: string | undefined;
  } | null>(null);
  const restoredConversation = useRef<string | undefined>(undefined);
  const reconciledRun = useRef<string | null>(null);
  const history = useQuery<ConversationHistory>({
    queryKey: ["messages", id],
    queryFn: () => api("/conversations/" + id + "/messages"),
    enabled: !!id,
  });
  useEffect(() => { if (projectId) setSelectedProject(projectId); }, [projectId]);
  const run = useQuery<Run>({
    queryKey: ["run", runId],
    queryFn: () => api("/runs/" + runId),
    enabled: !!runId,
    refetchInterval: (q) =>
      !streamConnected && (q.state.data?.status === "queued" || q.state.data?.status === "running" || q.state.data?.status === "waiting_approval" || q.state.data?.status === "paused")
        ? 1500 : false,
  });
  const runActive = run.data?.status === "queued" || run.data?.status === "running" || run.data?.status === "waiting_approval" || run.data?.status === "paused";
  const toolHistory = useQuery<ToolCallHistory[]>({
    queryKey: ["tool-calls", id],
    queryFn: () => api("/conversations/" + id + "/tool-calls"),
    enabled: !!id,
    refetchInterval: !streamConnected && runActive ? 5000 : false,
  });
  useEffect(() => {
    if (!runId || !run.data || streamConnected) return;
    if (!["completed", "failed", "cancelled", "interrupted"].includes(run.data.status)) return;
    const revision = `${runId}:${run.data.status}`;
    if (reconciledRun.current === revision) return;
    reconciledRun.current = revision;
    qc.invalidateQueries({ queryKey: ["messages", id] });
    qc.invalidateQueries({ queryKey: ["tool-calls", id] });
  }, [id, qc, run.data, runId, streamConnected]);
  useEffect(() => {
    restoredConversation.current = undefined;
    setRunId("");
    setEvents([]);
    setStreamText("");
    setReasoning("");
    setStreamConnected(false);
  }, [id]);
  useEffect(() => {
    const latestRun = history.data?.runs?.at(-1);
    if (latestRun) setRunId(latestRun.id);
    if (id && history.data && restoredConversation.current !== id) {
      restoredConversation.current = id;
      setSelectedProject((history.data as ConversationHistory & { project_id?: string }).project_id || "");
      const selection = history.data.selection;
      if (selection) {
        setModel(selection.model_id);
        setAgent(selection.agent_id || "");
        setMode(selection.mode || "chat");
        setResearchMode(Boolean((selection as { research_mode?: boolean }).research_mode));
      }
    }
  }, [history.data, id]);
  useEffect(() => {
    if (!runId) return;
    setEvents([]);
    setStreamText("");
    setReasoning("");
    const source = new EventSource("/api/v1/runs/" + runId + "/events");
    eventSourceRef.current = source;
    source.onopen = () => setStreamConnected(true);
    // A run is bounded by its budget on the server; if the stream never
    // terminates, bail out and let the polling fallback take over.
    const watchdog = setTimeout(() => {
      if (source.readyState === EventSource.OPEN) {
        source.close();
        setStreamConnected(false);
      }
    }, 15 * 60 * 1000);
    const finish = () => {
      clearTimeout(watchdog);
      source.close();
      setBusy(false);
      setStreamConnected(false);
      qc.invalidateQueries({ queryKey: ["messages", id] });
      qc.invalidateQueries({ queryKey: ["run", runId] });
    };
    const onActivity = (event: Event) => {
      if (!(event instanceof MessageEvent) || typeof event.data !== "string") return;
      const v = parseRunEvent(event.data);
      if (!v) return;
      if (v.kind === "text") setStreamText((t) => t + v.text);
      else if (v.kind === "reasoning") setReasoning((t) => t + v.text);
      else if (v.kind === "model_started") {
        setStreamText("");
        setReasoning((t) => (t ? t + "\n\n" : t));
        setEvents((xs) => [...xs, v]);
        // A new step was committed on the server; refresh steps/budget.
        qc.invalidateQueries({ queryKey: ["run", runId] });
      } else {
        setEvents((xs) => [...xs, v]);
        if (v.kind === "approval") {
          qc.invalidateQueries({ queryKey: ["run", runId] });
        }
        if (v.kind === "status" || v.kind === "context_summary") qc.invalidateQueries({ queryKey: ["run", runId] });
        if (v.kind === "message") {
          setStreamText("");
          qc.invalidateQueries({ queryKey: ["messages", id] });
        }
      }
      if (["tool_started", "tool_result", "plan"].includes(v.kind))
        qc.invalidateQueries({ queryKey: ["run", runId] });
      // update_plan mirrors the conversation todo list; refresh the pane.
      if (v.kind === "plan") qc.invalidateQueries({ queryKey: ["messages", id] });
      if (["tool_started", "tool_result", "approval", "status"].includes(v.kind))
        qc.invalidateQueries({ queryKey: ["tool-calls", id] });
      if (v.kind === "browser_frame") qc.invalidateQueries({ queryKey: ["browser-frames", runId] });
      if (
        v.kind === "checkpoint_saved" ||
        v.kind === "budget_extension_requested" ||
        v.kind === "budget_extension_resolved" ||
        v.kind === "stagnation_detected" ||
        v.kind === "verification_required" ||
        v.kind === "phase_advanced"
      ) {
        qc.invalidateQueries({ queryKey: ["run", runId] });
      }
    };
    source.addEventListener("activity", onActivity);
    source.addEventListener("done", finish);
    source.onerror = () => {
      clearTimeout(watchdog);
      source.close();
      setBusy(false);
      setStreamConnected(false);
      setError(new Error(ja.streamLost));
    };
    return () => {
      clearTimeout(watchdog);
      source.close();
      if (eventSourceRef.current === source) eventSourceRef.current = null;
    };
  }, [runId, id, qc]);
  useEffect(() => {
    bottom.current?.scrollIntoView({ behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  }, [streamText, history.data]);
  const active =
    run.data &&
    ["running", "queued", "waiting_approval", "paused"].includes(run.data.status);
  const live = liveToolCalls(events, runId);
  const currentRunMessageId = history.data?.runs?.find((r) => r.id === runId)?.message_id;
  async function send(e?: FormEvent) {
    e?.preventDefault();
    if ((!text.trim() && !attachments.length) || busy || active) return;
    setError(null);
    setBusy(true);
    try {
      const body = {
        content: text,
        model_id: model,
        mode: researchMode ? "agent" : mode,
        agent_id: agent,
        artifact_ids: attachments.map((a) => a.artifact_id),
        temporary_mode: temporaryMode,
        allow_tools: allowTools,
        research_mode: researchMode && !temporaryMode,
      };
      const signature = JSON.stringify({ id, ...body });
      if (!pendingSend.current || pendingSend.current.signature !== signature) {
        pendingSend.current = {
          signature,
          key: uuid(),
          conversation: id,
        };
      }
      let conversation = pendingSend.current.conversation;
      if (!conversation) {
        const c = await api<Row>("/conversations", "POST", { project_id: temporaryMode ? null : selectedProject || null });
        conversation = c.id;
        pendingSend.current.conversation = c.id;
      }
      const r = await api<SendMessage>(
        "/conversations/" + conversation + "/messages",
        "POST",
        body,
        pendingSend.current.key,
      );
      pendingSend.current = null;
      setText("");
      setAttachments([]);
      await qc.invalidateQueries({ queryKey: ["/conversations"] });
      if (id !== conversation) navigate("/chat/" + conversation);
      else {
        setRunId(r.run_id);
        qc.invalidateQueries({ queryKey: ["messages", id] });
      }
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  async function attach(files: File[]) {
    if (!files.length) return;
    setError(null);
    try {
      for (const file of files) {
        const f = new FormData();
        f.set("file", file);
        const a = await api<Artifact>("/artifacts", "POST", f);
        setAttachments((xs) => [...xs, a]);
      }
    } catch (e) {
      setError(e);
    }
  }
  const selected = models.data?.find((x) => x.id === model);
  const selectedAgent = agents.data?.find((a) => a.id === agent);
  const modeLabel = mode;
  const modelLabel = model === "auto" ? "Auto" : selected?.data.name || selected?.data.model_id || t("モデルを選択");
  const permissionCounts = (tools.data || []).reduce(
    (counts: Record<string, number>, tool) => {
      const rule = permissionRules.data
        ?.filter((r) => r.data.agent_id === agent && r.data.tool_id === tool.id)
        .at(-1);
      const value = rule?.data.permission || tool.default_permission || "allow";
      counts[value] = (counts[value] || 0) + 1;
      return counts;
    },
    {},
  );
  async function rate(messageId: string, current: Feedback, value: Exclude<Feedback, null>) {
    try {
      await api("/messages/" + messageId + "/feedback", "PUT", { value: current === value ? null : value });
      qc.invalidateQueries({ queryKey: ["messages", id] });
    } catch (e) {
      setError(e);
    }
  }
  return (
    <div className="chat-page">
      <div className="chat-body">
        <div className="conversation">
          {!id && !history.data?.messages?.length ? (
            <section className="welcome">
              <div className="welcome-icon">
                <Sparkles size={30} />
              </div>
              <p className="eyebrow">MIX</p>
              <h1>{t(welcome)}</h1>
              <div className="welcome-note">
                <ShieldCheck size={14} /> {t("必要なときだけ、確認のうえで作業を進めます")}
              </div>
            </section>
          ) : (
            <div className="messages">
              {history.data?.messages.map((m) => (
                <div key={m.id}>
                {m.data.role === "user" && history.data?.runs?.filter(r => r.message_id === m.id && r.context_summary?.text).map(r => (
                  <details className="context-summary" key={r.id}>
                    <summary>{t("この実行で文脈を圧縮しました ·")} {r.context_summary?.covered_count ?? 0}{t("件を要約")}</summary>
                    <p>{t("古い会話は以下の要約としてモデルに渡しました。元の会話は画面に残っています。")}</p>
                    <div className="context-summary-text">{r.context_summary?.text}</div>
                  </details>
                ))}
                <article className={"message " + m.data.role}>
                  <div className="message-label">
                    {m.data.role === "user" ? (
                      t("あなた")
                    ) : (
                      <>
                        <span className="mini-mark">M</span>MIX agent
                      </>
                    )}
                  </div>
                  <div className="markdown">
                    <Markdown remarkPlugins={[remarkGfm, remarkBreaks]}>
                      {m.data.content}
                    </Markdown>
                  </div>
                  {m.data.artifacts?.map((artifact) => (
                    <ArtifactCard key={artifact.artifact_id} artifact={artifact} />
                  )) || m.data.artifact_ids?.map((artifact_id: string) => (
                    <ArtifactCard key={artifact_id} artifact={{ artifact_id }} />
                  ))}
                  {m.data.performance && (
                    <div className="message-performance" title={`出力 ${m.data.performance.output_tokens} tokens / 生成 ${m.data.performance.generation_ms}ms`}>
                      {m.data.performance.tokens_per_second.toFixed(1)} tokens/s
                    </div>
                  )}
                  {m.data.auto_selection && (
                    <div className="message-auto">
                      <small>Auto: {m.data.auto_selection.model_name || m.data.auto_selection.model_id}</small>
                      <span>
                        <button className={m.data.feedback === "up" ? "selected" : ""} onClick={() => rate(m.id, m.data.feedback, "up")} aria-label={t("良い回答")}><ThumbsUp size={14} /></button>
                        <button className={m.data.feedback === "down" ? "selected" : ""} onClick={() => rate(m.id, m.data.feedback, "down")} aria-label={t("良くない回答")}><ThumbsDown size={14} /></button>
                      </span>
                    </div>
                  )}
                </article>
                {m.data.role === "user" && <RunActivity
                  running={active && currentRunMessageId === m.id}
                  calls={(() => {
                    const calls = (toolHistory.data || []).filter((call) =>
                      history.data?.runs?.find((run) => run.id === call.run_id)?.message_id === m.id,
                    );
                    if (!active || currentRunMessageId !== m.id) return calls;
                    const known = new Set(calls.map((call) => call.id));
                    return [...calls, ...live.filter((call) => !known.has(call.id))];
                  })()}
                  onApproval={async (approvalId, decision) => {
                    try {
                      await api("/approvals/" + approvalId + "/decision", "POST", { decision });
                      qc.invalidateQueries({ queryKey: ["tool-calls", id] });
                      qc.invalidateQueries({ queryKey: ["run", runId] });
                    } catch (e) { setError(e); }
                  }}
                />}
                {m.data.role === "user" && currentRunMessageId === m.id && (
                  <TodoPane items={history.data?.todos} />
                )}
                {m.data.role === "user" && history.data?.runs?.filter(r => r.message_id === m.id && toolHistory.data?.some(call => call.run_id === r.id && call.tool_name.startsWith("browser_"))).map(r => (
                  <BrowserPanel key={r.id} runId={r.id} run={r.id === runId ? run.data : undefined} onChange={() => {
                    qc.invalidateQueries({ queryKey: ["run", runId] });
                    qc.invalidateQueries({ queryKey: ["browser-frames", r.id] });
                  }} />
                ))}
                </div>
              ))}
              <ReasoningSummary text={reasoning} />
              {streamText && (
                <article className="message assistant">
                  <div className="message-label">
                    MIX agent <span className="pulse" />
                  </div>
                  <div className="markdown">
                    <Markdown remarkPlugins={[remarkGfm, remarkBreaks]}>
                      {streamText}
                    </Markdown>
                  </div>
                </article>
              )}
              {active && !streamText && (
                <p className="processing">
                  <span className="pulse" />
                  {t(run.data.status === "paused" ? "ブラウザを手動操作中" : run.data.status === "waiting_approval"
                    ? "操作の承認を待っています"
                    : "処理しています…")}
                </p>
              )}
              {run.data?.reason && (
                <div className="notice">{run.data.reason}</div>
              )}
              {run.data && active && (
                <RunBudget run={run.data} active={!!active} fetchedAt={run.dataUpdatedAt} />
              )}
              {run.data?.status === "interrupted" && (
                <CheckpointsPanel run={run.data} onError={setError} onResumed={() => {
                  qc.invalidateQueries({ queryKey: ["run", runId] });
                  qc.invalidateQueries({ queryKey: ["tool-calls", id] });
                  setRunId("");
                  setTimeout(() => setRunId(runId), 10);
                }} />
              )}
              {run.data?.status === "budget_extension_pending" && run.data.budget_extension_request && (
                <BudgetExtensionPanel
                  runId={runId}
                  request={run.data.budget_extension_request}
                  used={run.data.budget_extensions_used}
                  max={run.data.budget_extensions_max}
                  onError={setError}
                  onResolved={() => {
                    qc.invalidateQueries({ queryKey: ["run", runId] });
                  }}
                />
              )}
              {run.data?.stagnation && run.data.stagnation.length > 0 && (
                <StagnationPanel findings={run.data.stagnation as StagnationFinding[]} />
              )}
              <div ref={bottom} />
            </div>
          )}
          <div className="composer-wrap">
            <ErrorBox
              error={error || history.error || models.error || tools.error || permissionRules.error}
            />
            <form className="composer" onSubmit={send}>
              {temporaryMode && <p className="temporary-notice" role="status">{t("一時モード: Run終了後にMIX側の会話・回答・送信した添付を削除します。選択した添付は送信前にアップロードされます。Toolは")}{t(allowTools ? "許可中（外部への送信や副作用が残る場合があります）" : "無効")}。</p>}
              {attachments.length > 0 && (
                <div className="attachments">
                  {attachments.map((a) => (
                    <span key={a.artifact_id}>
                      <FileText size={13} />
                      {a.name}
                      <button
                        type="button"
                        aria-label={t("添付解除")}
                        onClick={() =>
                          setAttachments((xs) => xs.filter((x) => x !== a))
                        }
                      >
                        <X size={12} />
                      </button>
                    </span>
                  ))}
                </div>
              )}
              <textarea
                aria-label={t("メッセージ")}
                placeholder={t("MIX agent にメッセージを送信…")}
                value={text}
                onChange={(e) => setText(e.target.value)}
                onKeyDown={(e) => {
                  if (
                    e.key === "Enter" &&
                    !e.shiftKey &&
                    !e.nativeEvent.isComposing && e.nativeEvent.keyCode !== 229
                  ) {
                    e.preventDefault();
                    send();
                  }
                }}
              />
              <div className="composer-bottom">
                <details className="assistant-settings">
                  <summary>
                    <SlidersHorizontal size={15} />
                    <span className="assistant-settings-summary">{modelLabel} · {modeLabel}</span>
                  </summary>
                  <div className="assistant-settings-panel">
                    <div className="assistant-panel-head">
                      <span className="assistant-panel-icon"><SlidersHorizontal size={15} /></span>
                      <span className="assistant-panel-title">
                        <b>{t("アシスタント設定")}</b>
                        <small>{modelLabel} · {modeLabel}</small>
                      </span>
                    </div>
                    <div className="assistant-sections">
                      <section className="assistant-section">
                        <h4 className="assistant-section-title">{t("モデルと応答方法")}</h4>
                        <div className="assistant-setting-row">
                          <span>{t("モデル")}</span>
                          <ModelPicker models={models.data} value={model} onChange={setModel} temporaryMode={temporaryMode} allowTools={allowTools} onTemporaryModeChange={value => { setTemporaryMode(value); if (value) setResearchMode(false); }} onAllowToolsChange={setAllowTools} />
                        </div>
                        <div className="assistant-setting-row mode-row">
                          <span>{t("応答方法")}</span>
                          <div className="mode-switch">
                            {modes.map((m) => (
                              <button type="button" key={m} disabled={active} className={mode === m ? "selected" : ""} aria-pressed={mode === m} onClick={() => { setMode(m); if (m !== "agent") setResearchMode(false); }}>
                                {m === "chat" ? <MessageSquare size={13} /> : m === "thinking" ? <Brain size={13} /> : <Bot size={13} />}
                                {m}
                              </button>
                            ))}
                          </div>
                        </div>
                      </section>
                      <section className="assistant-section">
                        <h4 className="assistant-section-title">{t("文脈")}</h4>
                        <div className="assistant-setting-row">
                          <span>{t("プロジェクト")}</span>
                          <SelectMenu
                            value={selectedProject}
                            disabled={!!id || temporaryMode}
                            icon={<FolderKanban size={14} />}
                            ariaLabel={t("プロジェクト")}
                            onChange={setSelectedProject}
                            options={[{ value: "", label: t("なし") }, ...(projects.data || []).map(project => ({ value: project.id, label: project.data.name }))]}
                          />
                        </div>
                        <div className="assistant-setting-row assistant-toggle-row">
                          <span className="assistant-toggle-label">
                            <span>{t("調査モード")}</span>
                            <small id="research-mode-hint">{t("Webを調べ、出典つきレポートを作成")}</small>
                          </span>
                          <Toggle
                            checked={researchMode}
                            disabled={active || temporaryMode}
                            label={t("調査モード")}
                            describedBy="research-mode-hint"
                            onChange={checked => { setResearchMode(checked); if (checked) setMode("agent"); }}
                          />
                        </div>
                      </section>
                      <section className="assistant-section">
                        <h4 className="assistant-section-title">{t("アシスタントと権限")}</h4>
                        <div className="assistant-setting-row">
                          <span>{t("アシスタント")}</span>
                          <SelectMenu
                            value={agent}
                            icon={<UserCog size={14} />}
                            ariaLabel={t("アシスタント")}
                            onChange={(nextAgent) => {
                              setAgent(nextAgent);
                              const next = agents.data?.find((x) => x.id === nextAgent);
                              if (next) {
                                setMode(next.data.mode);
                                if (next.data.mode !== "agent") setResearchMode(false);
                                if (next.data.model_id) setModel(next.data.model_id);
                              }
                            }}
                            options={[{ value: "", label: t("標準") }, ...(agents.data || []).map(a => ({ value: a.id, label: a.data.name }))]}
                          />
                        </div>
                        <div className="assistant-permissions" title={t("現在のアシスタントに適用されるTool権限")}>
                          <ShieldCheck size={14} />
                          <span>{t("ツール権限: 許可")} {permissionCounts.allow || 0} {t("· 確認")} {permissionCounts.ask || 0} {t("· 拒否")} {permissionCounts.deny || 0}</span>
                        </div>
                        <ModeStatus mode={mode} model={selected} agent={selectedAgent} budget={active && run.data?.mode === mode ? run.data.budget : undefined} />
                      </section>
                    </div>
                  </div>
                </details>
                <div className="composer-actions">
                  <button
                    type="button"
                    className="icon"
                    aria-label={t("添付")}
                    onClick={() => fileRef.current?.click()}
                  >
                    <Paperclip size={18} />
                  </button>
                  <input
                    ref={fileRef}
                    hidden
                    type="file"
                    multiple
                    accept="image/*,.txt,.md,.pdf"
                    onChange={(e) => {
                      const files = Array.from(e.currentTarget.files || []);
                      e.currentTarget.value = "";
                      attach(files);
                    }}
                  />
                  {active ? (
                    <button
                      type="button"
                      className="send"
                      aria-label={t("停止")}
                      onClick={() =>
                        api("/runs/" + runId + "/cancel", "POST").catch(
                          setError,
                        )
                      }
                    >
                      <Square size={15} />
                    </button>
                  ) : (
                    <button
                      className="send"
                      aria-label={t("送信")}
                      disabled={
                        busy || (!text.trim() && !attachments.length)
                      }
                    >
                      <ArrowUp size={19} />
                    </button>
                  )}
                </div>
              </div>
            </form>
            <p className="composer-foot">
              {t("MIX agent の回答には誤りが含まれることがあります。重要な情報は確認してください。")}
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
