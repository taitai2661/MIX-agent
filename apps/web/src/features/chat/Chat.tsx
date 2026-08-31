import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Activity,
  ArrowUp,
  Bot,
  Brain,
  ChevronRight,
  FileText,
  Globe,
  MessageSquare,
  Paperclip,
  ShieldCheck,
  Sparkles,
  Square,
  Terminal,
  ThumbsDown,
  ThumbsUp,
  X,
} from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import Markdown from "react-markdown";
import { useNavigate, useParams } from "react-router-dom";
import remarkGfm from "remark-gfm";

import { Empty, ErrorBox, useRows } from "@/components/shared";
import { ModelPicker } from "@/components/model-picker";
import { ModeStatus, ReasoningSummary } from "./ModeStatus";
import { ToolActivity, type ActivitySummary } from "./ToolActivity";
import { ArtifactCard, type ChatArtifact } from "./ArtifactCard";

export function Chat() {
  const { id } = useParams();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const models = useRows("/models"),
    agents = useRows("/agents"),
    tools = useQuery<any[]>({
      queryKey: ["/tools"],
      queryFn: () => api("/tools"),
    }),
    permissionRules = useRows("/permission-rules");
  const setting = useQuery({
    queryKey: ["/settings"],
    queryFn: () => api("/settings"),
  });
  const [model, setModel] = useState(""),
    [mode, setMode] = useState("chat"),
    [agent, setAgent] = useState(""),
    [text, setText] = useState(""),
    [error, setError] = useState<unknown>(null),
    [busy, setBusy] = useState(false),
    [attachments, setAttachments] = useState<any[]>([]),
    [runId, setRunId] = useState(""),
    [streamText, setStreamText] = useState(""),
    [reasoning, setReasoning] = useState(""),
    [events, setEvents] = useState<any[]>([]),
    [approvalEvents, setApprovalEvents] = useState<any[]>([]),
    [activeActivity, setActiveActivity] = useState<ActivitySummary | null>(null),
    [showActivity, setShowActivity] = useState(
      () => window.matchMedia("(min-width:1100px)").matches,
    );
  const fileRef = useRef<HTMLInputElement>(null),
    bottom = useRef<HTMLDivElement>(null);
  const pendingSend = useRef<{
    signature: string;
    key: string;
    conversation: string | undefined;
  } | null>(null);
  const restoredConversation = useRef<string | undefined>(undefined);
  const history = useQuery<any>({
    queryKey: ["messages", id],
    queryFn: () => api("/conversations/" + id + "/messages"),
    enabled: !!id,
  });
  const run = useQuery<any>({
    queryKey: ["run", runId],
    queryFn: () => api("/runs/" + runId),
    enabled: !!runId,
    refetchInterval: (q) =>
      ["queued", "running", "waiting_approval"].includes(q.state.data?.status)
        ? 1500
        : false,
  });
  useEffect(() => {
    if (!model && setting.data?.data.default_model_id)
      setModel(setting.data.data.default_model_id);
  }, [setting.data, model]);
  useEffect(() => {
    restoredConversation.current = undefined;
    setRunId("");
    setEvents([]);
    setStreamText("");
    setReasoning("");
    setApprovalEvents([]);
    setActiveActivity(null);
  }, [id]);
  useEffect(() => {
    if (history.data?.runs?.length) setRunId(history.data.runs.at(-1).id);
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
    source.addEventListener("activity", (e: any) => {
      const v = JSON.parse(e.data);
      if (v.kind === "text") setStreamText((t) => t + v.text);
      else if (v.kind === "reasoning") setReasoning((t) => t + v.text);
      else if (v.kind === "model_started") {
        setStreamText("");
        setReasoning((t) => (t ? t + "\n\n" : t));
        setEvents((xs) => [...xs, v]);
      } else {
        setEvents((xs) => [...xs, v]);
        if (v.kind === "approval") {
          setShowActivity(true);
          setApprovalEvents((xs) => [...xs.filter((x) => x.id !== v.id), v]);
          qc.invalidateQueries({ queryKey: ["run", runId] });
        }
        if (v.kind === "message") {
          setStreamText("");
          qc.invalidateQueries({ queryKey: ["messages", id] });
        }
      }
      if (v.kind === "tool_started" && v.activity) setActiveActivity(v.activity);
      if (["tool_result", "model_started", "message"].includes(v.kind) ||
          (v.kind === "status" && v.status !== "running")) setActiveActivity(null);
    });
    source.addEventListener("done", () => {
      source.close();
      qc.invalidateQueries({ queryKey: ["messages", id] });
      qc.invalidateQueries({ queryKey: ["run", runId] });
    });
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
        const c = await api("/conversations", "POST", {});
        conversation = c.id;
        pendingSend.current.conversation = c.id;
      }
      const r = await api(
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
        const a = await api("/artifacts", "POST", f);
        setAttachments((xs) => [...xs, a]);
      }
    } catch (e) {
      setError(e);
    }
  }
  const selected = models.data?.find((x) => x.id === model);
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
  async function rate(messageId: string, current: string | undefined, value: "up" | "down") {
    try {
      await api("/messages/" + messageId + "/feedback", "PUT", { value: current === value ? null : value });
      qc.invalidateQueries({ queryKey: ["messages", id] });
    } catch (e) {
      setError(e);
    }
  }
  return (
    <div className="chat-page">
      <div className="chat-toolbar">
        <ModelPicker models={models.data} value={model} onChange={setModel} />
        <span className="subtle">
          {selected ? "接続済みの設定" : "設定からProviderを追加してください"}
        </span>
        <span className="permission-summary" title="現在のAgentに適用されるTool権限">
          <ShieldCheck size={14} />
          権限: 許可 {permissionCounts.allow || 0} · 確認 {permissionCounts.ask || 0} · 拒否 {permissionCounts.deny || 0}
        </span>
        <button
          className="button toolbar-action"
          onClick={() => setShowActivity(!showActivity)}
          aria-pressed={showActivity}
        >
          <Activity size={16} />
          その他のアクション
        </button>
      </div>
      <div className="chat-body">
        <div className="conversation">
          {!id && !history.data?.messages?.length ? (
            <section className="welcome">
              <div className="welcome-icon">
                <Sparkles size={30} />
              </div>
              <p className="eyebrow">A LITTLE HELP. MORE POSSIBILITIES.</p>
              <h1>今日は、何を進めましょう？</h1>
              <div className="suggestions">
                {[
                  ["考えを整理する", "アイデアを一緒に整理してください", Brain],
                  [
                    "調べて、理解する",
                    "知りたいテーマを分かりやすく説明してください",
                    Globe,
                  ],
                  [
                    "作業を任せる",
                    "作りたいものの実装計画を立ててください",
                    Terminal,
                  ],
                ].map(([title, prompt, Icon]: any) => (
                  <button key={title} onClick={() => setText(prompt)}>
                    <Icon size={19} />
                    <b>{title}</b>
                    <ChevronRight size={15} />
                  </button>
                ))}
              </div>
              <div className="welcome-note">
                <ShieldCheck size={14} />{" "}
                Toolsは設定された権限の範囲で動作します
              </div>
            </section>
          ) : (
            <div className="messages">
              {history.data?.messages.map((m: Row) => (
                <article className={"message " + m.data.role} key={m.id}>
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
                  {(m.data.artifacts as ChatArtifact[] | undefined)?.map((artifact) => (
                    <ArtifactCard key={artifact.artifact_id} artifact={artifact} />
                  )) || m.data.artifact_ids?.map((artifact_id: string) => (
                    <ArtifactCard key={artifact_id} artifact={{ artifact_id }} />
                  ))}
                  {m.data.auto_selection && (
                    <div className="message-auto">
                      <small>Auto: {m.data.auto_selection.model_id} · {m.data.auto_selection.reason}</small>
                      <span>
                        <button className={m.data.feedback === "up" ? "selected" : ""} onClick={() => rate(m.id, m.data.feedback, "up")} aria-label="良い回答"><ThumbsUp size={14} /></button>
                        <button className={m.data.feedback === "down" ? "selected" : ""} onClick={() => rate(m.id, m.data.feedback, "down")} aria-label="良くない回答"><ThumbsDown size={14} /></button>
                      </span>
                    </div>
                  )}
                </article>
              ))}
              <ReasoningSummary text={reasoning} />
              {events
                .filter((event) => event.kind === "tool_result" && event.activity)
                .map((event, index) => (
                  <ToolActivity key={event.id || index} activity={event.activity} />
                ))}
              {activeActivity && <ToolActivity activity={activeActivity} running />}
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
              {active && !streamText && !activeActivity && (
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
            <ModeStatus
              mode={mode}
              model={selected}
              agent={agents.data?.find((a) => a.id === agent)}
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
                <div className="mode-switch">
                  {["chat", "thinking", "agent"].map((m) => (
                    <button
                      type="button"
                      key={m}
                      className={mode === m ? "selected" : ""}
                      aria-pressed={mode === m}
                      onClick={() => setMode(m)}
                    >
                      {m === "chat" ? (
                        <MessageSquare size={13} />
                      ) : m === "thinking" ? (
                        <Brain size={13} />
                      ) : (
                        <Bot size={13} />
                      )}{" "}
                      {m[0].toUpperCase() + m.slice(1)}
                    </button>
                  ))}
                </div>
                <select
                  className="agent-select"
                  aria-label="Agent"
                  value={agent}
                  onChange={(e) => {
                    setAgent(e.target.value);
                    const a = agents.data?.find((x) => x.id === e.target.value);
                    if (a) {
                      setMode(a.data.mode);
                      if (a.data.model_id) setModel(a.data.model_id);
                    }
                  }}
                >
                  <option value="">標準</option>
                  {agents.data?.map((a) => (
                    <option key={a.id} value={a.id}>
                      {a.data.name}
                    </option>
                  ))}
                </select>
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
                        busy || !model || (!text.trim() && !attachments.length)
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
        {id && showActivity && (
          <aside className="activity">
            <div className="activity-title">
              <Activity size={17} />
              <b>Activity</b>
              <span>{run.data?.status || "ready"}</span>
            </div>
            {!events.length && (
              <Empty>Toolの実行や計画がここに表示されます。</Empty>
            )}
            {[...(run.data?.approvals || []), ...approvalEvents]
              .filter((a: any, index: number, xs: any[]) =>
                a.status === "pending" && xs.findIndex((x) => x.id === a.id) === index,
              )
              .map((a: any) => (
                <div className="approval" key={a.id}>
                  <ShieldCheck size={20} />
                  <b>承認が必要です</b>
                  <p>{a.data.tool}</p>
                  <pre>{JSON.stringify(a.data.arguments, null, 2)}</pre>
                  <small>許可範囲: {JSON.stringify(a.data.scope)}</small>
                  <div className="approval-buttons">
                    {[
                      ["once", "今回のみ"],
                      ["always", "常に許可"],
                      ["denied", "拒否"],
                    ].map(([value, label]) => (
                      <Button
                        key={value}
                        variant={value === "once" ? "default" : "outline"}
                        onClick={async () => {
                          try {
                            await api(
                              "/approvals/" + a.id + "/decision",
                              "POST",
                              { decision: value },
                            );
                            qc.invalidateQueries({ queryKey: ["run", runId] });
                          } catch (e) {
                            setError(e);
                          }
                        }}
                      >
                        {label}
                      </Button>
                    ))}
                  </div>
                </div>
              ))}
            {events
              .filter((e) => !["message", "approval"].includes(e.kind))
              .map((e, i) => (
                <details key={i} className="activity-item">
                  <summary>
                    <span className={"event-dot " + e.kind} />
                    {e.name ||
                      (
                        {
                          model_started: "モデル呼び出し",
                          model_selected: "Autoモデルを選択",
                          model_rerouted: "Autoモデルを再試行",
                          plan: "Plan",
                          status: "実行状態",
                        } as any
                      )[e.kind] ||
                      e.kind}
                    <ChevronRight size={13} />
                  </summary>
                  <pre>{JSON.stringify(e.result || e, null, 2)}</pre>
                  {e.result?.artifact && (
                    <a
                      href={
                        "/api/v1/artifacts/" + e.result.artifact.artifact_id
                      }
                    >
                      成果物をダウンロード
                    </a>
                  )}
                </details>
              ))}
          </aside>
        )}
      </div>
    </div>
  );
}
