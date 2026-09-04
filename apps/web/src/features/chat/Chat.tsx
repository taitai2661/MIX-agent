import { api, type Row } from "@/app/api";
import type { components } from "@/generated/api";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowUp,
  Bot,
  Brain,
  FileText,
  MessageSquare,
  Paperclip,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Square,
  ThumbsDown,
  ThumbsUp,
  X,
} from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import Markdown from "react-markdown";
import { useNavigate, useParams } from "react-router-dom";
import remarkGfm from "remark-gfm";

import { ErrorBox, useRows } from "@/components/shared";
import { ModelPicker } from "@/components/model-picker";
import { ModeStatus, ReasoningSummary } from "./ModeStatus";
import { ToolHistory, type ToolCallHistory } from "./ToolHistory";
import { ArtifactCard } from "./ArtifactCard";
import {
  parseRunEvent,
  type Artifact,
  type ChatMode,
  type ConversationHistory,
  type PermissionRule,
  type Run,
  type RunEvent,
  type SendMessage,
  type Tool,
} from "./types";

type ChatModel = Row & { data: { capabilities?: Record<string, boolean | null>; overrides?: Record<string, boolean | null>; reasoning_control?: boolean; tool_probe?: { status?: string } } };
type ChatAgent = Row & { data: { mode: ChatMode; model_id?: string; name: string; tool_ids?: string[] } };
type Feedback = components["schemas"]["FeedbackInput"]["value"];
const modes = ["chat", "thinking", "agent"] as const satisfies readonly ChatMode[];

export function Chat() {
  const { id } = useParams();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const models = useRows<ChatModel>("/models"),
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
    [agent, setAgent] = useState(""),
    [text, setText] = useState(""),
    [error, setError] = useState<unknown>(null),
    [busy, setBusy] = useState(false),
    [attachments, setAttachments] = useState<Artifact[]>([]),
    [runId, setRunId] = useState(""),
    [streamText, setStreamText] = useState(""),
    [reasoning, setReasoning] = useState(""),
    [events, setEvents] = useState<RunEvent[]>([]);
  const fileRef = useRef<HTMLInputElement>(null),
    bottom = useRef<HTMLDivElement>(null);
  const pendingSend = useRef<{
    signature: string;
    key: string;
    conversation: string | undefined;
  } | null>(null);
  const restoredConversation = useRef<string | undefined>(undefined);
  const history = useQuery<ConversationHistory>({
    queryKey: ["messages", id],
    queryFn: () => api("/conversations/" + id + "/messages"),
    enabled: !!id,
  });
  const run = useQuery<Run>({
    queryKey: ["run", runId],
    queryFn: () => api("/runs/" + runId),
    enabled: !!runId,
    refetchInterval: (q) =>
      q.state.data?.status === "queued" || q.state.data?.status === "running" || q.state.data?.status === "waiting_approval"
        ? 1500 : false,
  });
  const toolHistory = useQuery<ToolCallHistory[]>({
    queryKey: ["tool-calls", id],
    queryFn: () => api("/conversations/" + id + "/tool-calls"),
    enabled: !!id,
    refetchInterval: run.data?.status === "queued" || run.data?.status === "running" || run.data?.status === "waiting_approval" ? 1500 : false,
  });
  useEffect(() => {
    restoredConversation.current = undefined;
    setRunId("");
    setEvents([]);
    setStreamText("");
    setReasoning("");
  }, [id]);
  useEffect(() => {
    const latestRun = history.data?.runs?.at(-1);
    if (latestRun) setRunId(latestRun.id);
    if (id && history.data && restoredConversation.current !== id) {
      restoredConversation.current = id;
      const selection = history.data.selection;
      if (selection) {
        setModel(selection.model_id);
        setAgent(selection.agent_id || "");
        setMode(selection.mode || "chat");
      }
    }
  }, [history.data, id]);
  useEffect(() => {
    if (!runId) return;
    setEvents([]);
    setStreamText("");
    setReasoning("");
    const source = new EventSource("/api/v1/runs/" + runId + "/events");
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
      } else {
        setEvents((xs) => [...xs, v]);
        if (v.kind === "approval") {
          qc.invalidateQueries({ queryKey: ["run", runId] });
        }
        if (v.kind === "message") {
          setStreamText("");
          qc.invalidateQueries({ queryKey: ["messages", id] });
        }
      }
      if (["tool_started", "tool_result", "approval", "status"].includes(v.kind))
        qc.invalidateQueries({ queryKey: ["tool-calls", id] });
    };
    source.addEventListener("activity", onActivity);
    source.addEventListener("done", () => {
      source.close();
      setBusy(false);
      qc.invalidateQueries({ queryKey: ["messages", id] });
      qc.invalidateQueries({ queryKey: ["run", runId] });
    });
    source.onerror = () => {
      source.close();
      setBusy(false);
      setError(new Error("ストリーム接続が切れました。再試行してください。"));
    };
    return () => source.close();
  }, [runId, id, qc]);
  useEffect(() => {
    bottom.current?.scrollIntoView({ behavior: "smooth" });
  }, [streamText, history.data]);
  const active =
    run.data &&
    ["running", "queued", "waiting_approval"].includes(run.data.status);
  async function send(e?: FormEvent) {
    e?.preventDefault();
    if ((!text.trim() && !attachments.length) || busy || active) return;
    setError(null);
    setBusy(true);
    try {
      const body = {
        content: text,
        model_id: model,
        mode,
        agent_id: agent,
        artifact_ids: attachments.map((a) => a.artifact_id),
        acknowledge_unknown_capability: false,
        temporary_mode: temporaryMode,
        allow_tools: allowTools,
      };
      const signature = JSON.stringify({ id, ...body });
      if (!pendingSend.current || pendingSend.current.signature !== signature) {
        pendingSend.current = {
          signature,
          key: crypto.randomUUID(),
          conversation: id,
        };
      }
      let conversation = pendingSend.current.conversation;
      if (!conversation) {
        const c = await api<Row>("/conversations", "POST", {});
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
  async function attach(files: FileList | null) {
    if (!files) return;
    setError(null);
    try {
      for (const file of Array.from(files)) {
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
  const modelLabel = model === "auto" ? "Auto" : selected?.data.name || selected?.data.model_id || "モデルを選択";
  const permissionCounts = (tools.data || []).reduce(
    (counts: Record<string, number>, tool) => {
      const rule = permissionRules.data
        ?.filter((r) => r.data.agent_id === agent && r.data.tool_id === tool.id)
        .at(-1);
      const value = rule?.data.permission || tool.default_permission || "ask";
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
              <h1>何をお手伝いできますか？</h1>
              <div className="welcome-note">
                <ShieldCheck size={14} /> 必要なときだけ、確認のうえで作業を進めます
              </div>
            </section>
          ) : (
            <div className="messages">
              {history.data?.messages.map((m) => (
                <div key={m.id}>
                <article className={"message " + m.data.role}>
                  <div className="message-label">
                    {m.data.role === "user" ? (
                      "あなた"
                    ) : (
                      <>
                        <span className="mini-mark">M</span>MIX agent
                      </>
                    )}
                  </div>
                  <div className="markdown">
                    <Markdown remarkPlugins={[remarkGfm]}>
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
                      <small>Auto: {m.data.auto_selection.model_id}</small>
                      <span>
                        <button className={m.data.feedback === "up" ? "selected" : ""} onClick={() => rate(m.id, m.data.feedback, "up")} aria-label="良い回答"><ThumbsUp size={14} /></button>
                        <button className={m.data.feedback === "down" ? "selected" : ""} onClick={() => rate(m.id, m.data.feedback, "down")} aria-label="良くない回答"><ThumbsDown size={14} /></button>
                      </span>
                    </div>
                  )}
                </article>
                {m.data.role === "user" && <ToolHistory
                  calls={(toolHistory.data || []).filter((call) =>
                    history.data?.runs?.find((run) => run.id === call.run_id)?.message_id === m.id,
                  )}
                  onApproval={async (approvalId, decision) => {
                    try {
                      await api("/approvals/" + approvalId + "/decision", "POST", { decision });
                      qc.invalidateQueries({ queryKey: ["tool-calls", id] });
                      qc.invalidateQueries({ queryKey: ["run", runId] });
                    } catch (e) { setError(e); }
                  }}
                />}
                </div>
              ))}
              <ReasoningSummary text={reasoning} />
              {streamText && (
                <article className="message assistant">
                  <div className="message-label">
                    MIX agent <span className="pulse" />
                  </div>
                  <div className="markdown">
                    <Markdown remarkPlugins={[remarkGfm]}>
                      {streamText}
                    </Markdown>
                  </div>
                </article>
              )}
              {active && !streamText && (
                <p className="processing">
                  <span className="pulse" />
                  {run.data.status === "waiting_approval"
                    ? "操作の承認を待っています"
                    : "処理しています…"}
                </p>
              )}
              {run.data?.reason && (
                <div className="notice">{run.data.reason}</div>
              )}
              {run.data && active && (
                <p className="run-budget" role="status">
                  {run.data.mode}
                  {" · 残り "}{run.data.remaining.max_seconds ?? 0}秒
                  {" · "}{run.data.remaining.max_steps ?? 0}ステップ
                  {" · "}{run.data.remaining.max_tool_calls ?? 0} Tool Call
                </p>
              )}
              {run.data?.status === "interrupted" && (
                <Button
                  variant="outline"
                  onClick={async () => {
                    if (
                      confirm(
                        "結果不明の操作は再実行しません。外部状態を確認してから続けてください。",
                      )
                    ) {
                      try {
                        await api("/runs/" + runId + "/resume", "POST", {
                          acknowledge_unknown_result: true,
                        });
                        qc.invalidateQueries({ queryKey: ["run", runId] });
                        setRunId("");
                        setTimeout(() => setRunId(runId), 10);
                      } catch (e) {
                        setError(e);
                      }
                    }
                  }}
                >
                  確認して再開
                </Button>
              )}
              <div ref={bottom} />
            </div>
          )}
          <div className="composer-wrap">
            <ErrorBox
              error={error || history.error || models.error || tools.error || permissionRules.error}
            />
            <form className="composer" onSubmit={send}>
              {attachments.length > 0 && (
                <div className="attachments">
                  {attachments.map((a) => (
                    <span key={a.artifact_id}>
                      <FileText size={13} />
                      {a.name}
                      <button
                        type="button"
                        aria-label="添付解除"
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
                aria-label="メッセージ"
                placeholder="MIX agent にメッセージを送信…"
                value={text}
                onChange={(e) => setText(e.target.value)}
                onKeyDown={(e) => {
                  if (
                    e.key === "Enter" &&
                    !e.shiftKey &&
                    !e.nativeEvent.isComposing
                  ) {
                    e.preventDefault();
                    send();
                  }
                }}
              />
              <div className="composer-bottom">
                <details className="assistant-settings">
                  <summary><SlidersHorizontal size={15} /> {modelLabel} · {modeLabel}</summary>
                  <div className="assistant-settings-panel">
                    <div className="assistant-setting-row">
                      <span>モデル</span>
                      <ModelPicker models={models.data} value={model} onChange={setModel} temporaryMode={temporaryMode} allowTools={allowTools} onTemporaryModeChange={setTemporaryMode} onAllowToolsChange={setAllowTools} />
                    </div>
                    <div className="assistant-setting-row mode-row">
                      <span>応答方法</span>
                      <div className="mode-switch">
                        {modes.map((m) => (
                          <button type="button" key={m} disabled={active} className={mode === m ? "selected" : ""} aria-pressed={mode === m} onClick={() => setMode(m)}>
                            {m === "chat" ? <MessageSquare size={13} /> : m === "thinking" ? <Brain size={13} /> : <Bot size={13} />}
                            {m}
                          </button>
                        ))}
                      </div>
                    </div>
                    <label className="assistant-setting-row">
                      <span>アシスタント</span>
                      <select className="agent-select" value={agent} onChange={(e) => {
                        setAgent(e.target.value);
                        const next = agents.data?.find((x) => x.id === e.target.value);
                        if (next) {
                          setMode(next.data.mode);
                          if (next.data.model_id) setModel(next.data.model_id);
                        }
                      }}>
                        <option value="">標準</option>
                        {agents.data?.map((a) => <option key={a.id} value={a.id}>{a.data.name}</option>)}
                      </select>
                    </label>
                    <div className="assistant-permissions" title="現在のアシスタントに適用されるTool権限">
                      <ShieldCheck size={14} />
                      <span>ツール権限: 許可 {permissionCounts.allow || 0} · 確認 {permissionCounts.ask || 0} · 拒否 {permissionCounts.deny || 0}</span>
                    </div>
                    <ModeStatus mode={mode} model={selected} agent={selectedAgent} />
                  </div>
                </details>
                <div className="composer-actions">
                  <button
                    type="button"
                    className="icon"
                    aria-label="添付"
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
                    onChange={(e) => attach(e.target.files)}
                  />
                  {active ? (
                    <button
                      type="button"
                      className="send"
                      aria-label="停止"
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
                      aria-label="送信"
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
              MIX agent
              の回答には誤りが含まれることがあります。重要な情報は確認してください。
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
