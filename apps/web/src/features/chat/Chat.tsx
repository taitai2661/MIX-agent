import { api, type Row } from "@/app/api";
import { t } from "@/app/i18n";
import { uuid } from "@/app/uuid";
import { ja } from "@/app/strings";
import type { components } from "@/generated/api";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowDown,
  ArrowUp,
  Bot,
  Brain,
  CornerDownLeft,
  FileText,
  FolderKanban,
  MessageSquare,
  Paperclip,
  Pencil,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Square,
  ThumbsDown,
  ThumbsUp,
  UserCog,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import Markdown from "react-markdown";
import { useNavigate, useParams } from "react-router-dom";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

import { ErrorBox, useRows } from "@/components/shared";
import { ModelPicker } from "@/components/model-picker";
import { SelectMenu } from "@/components/select-menu";
import { Toggle } from "@/components/toggle";
import { useToast } from "@/components/toast";
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
import {
  nextVisit,
  welcomeTitle,
  welcomeNote,
  greetingText,
  suggestionLabelId,
  suggestionPromptId,
  type Welcome,
  type WorkspaceSnapshot,
} from "./greeting";
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

type RunStatus = "queued" | "running" | "waiting_approval" | "paused" | "budget_extension_pending" | "completed" | "failed" | "cancelled" | "interrupted";
const ACTIVE_RUN_STATUSES: readonly RunStatus[] = [
  "queued",
  "running",
  "waiting_approval",
  "paused",
  "budget_extension_pending",
];
const TERMINAL_RUN_STATUSES: readonly RunStatus[] = [
  "completed",
  "failed",
  "cancelled",
  "interrupted",
];

export function Chat({ workspace }: { workspace: WorkspaceSnapshot | null }) {
  const { id, projectId } = useParams();
  const draftKey = `mix:web-draft:${id ?? "new"}`;
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { addToast } = useToast();
  const models = useRows<ChatModel>("/models"),
    projects = useRows<Row>("/projects"),
    agents = useRows<ChatAgent>("/agents"),
    tools = useQuery<Tool[]>({
      queryKey: ["/tools"],
      queryFn: () => api("/tools"),
    }),
    permissionRules = useRows<PermissionRule>("/permission-rules");
  const settings = useQuery<{ data: { default_model_id?: string } }>({
    queryKey: ["/settings"],
    queryFn: () => api("/settings"),
  });
  const [model, setModel] = useState("auto"),
    [temporaryMode, setTemporaryMode] = useState(false),
    [allowTools, setAllowTools] = useState(false),
    [mode, setMode] = useState<ChatMode>("chat"),
    [researchMode, setResearchMode] = useState(false),
    [selectedProject, setSelectedProject] = useState(projectId || ""),
    [agent, setAgent] = useState(""),
    [text, setText] = useState(() => {
      try {
        return sessionStorage.getItem(draftKey) || "";
      } catch {
        return "";
      }
    }),
    [error, setError] = useState<unknown>(null),
    [busy, setBusy] = useState(false),
    [attachments, setAttachments] = useState<Artifact[]>([]),
    [uploading, setUploading] = useState(false),
    [runId, setRunId] = useState(""),
    [streamText, setStreamText] = useState(""),
    [reasoning, setReasoning] = useState(""),
    [events, setEvents] = useState<RunEvent[]>([]),
    [streamConnected, setStreamConnected] = useState(false);
  // Inline edit of a sent user turn, and the regeneration it triggers.
  const [editingId, setEditingId] = useState<string | null>(null),
    [editText, setEditText] = useState(""),
    [savingEdit, setSavingEdit] = useState(false);
  const [welcome, setWelcome] = useState<Welcome | null>(null);
  // `nextVisit` advances the stored visit count, so guard against React
  // StrictMode's double-invoked effect: without this the count (and the recent
  // fragment list) would be bumped twice per mount.
  const welcomePicked = useRef(false);
  const fileRef = useRef<HTMLInputElement>(null),
    composer = useRef<HTMLTextAreaElement>(null);
  const messagesRef = useRef<HTMLDivElement>(null);
  // True while the transcript is pinned to the newest message. Streaming must
  // not yank the viewport away from a user who scrolled up to read history.
  const [atBottom, setAtBottom] = useState(true);
  const atBottomRef = useRef(true);
  const eventSourceRef = useRef<EventSource | null>(null);
  // SSE can deliver dozens of text fragments per second.  Buffer them in refs
  // and flush once per animation frame instead of re-rendering the whole
  // transcript for every event.
  const streamTextBuffer = useRef("");
  const reasoningBuffer = useRef("");
  const streamFrame = useRef<number | null>(null);
  function flushStreamBuffers() {
    if (streamFrame.current !== null) {
      cancelAnimationFrame(streamFrame.current);
      streamFrame.current = null;
    }
    setStreamText(streamTextBuffer.current);
    setReasoning(reasoningBuffer.current);
  }
  function scheduleStreamFlush() {
    if (streamFrame.current !== null) return;
    streamFrame.current = requestAnimationFrame(() => {
      streamFrame.current = null;
      setStreamText(streamTextBuffer.current);
      setReasoning(reasoningBuffer.current);
    });
  }
  function resetStreamBuffers() {
    if (streamFrame.current !== null) {
      cancelAnimationFrame(streamFrame.current);
      streamFrame.current = null;
    }
    streamTextBuffer.current = "";
    reasoningBuffer.current = "";
    setStreamText("");
    setReasoning("");
  }
  const pendingSend = useRef<{
    signature: string;
    key: string;
    conversation: string | undefined;
  } | null>(null);
  const restoredConversation = useRef<string | undefined>(undefined);
  const modelInitializedFor = useRef<string | null>(null);
  const reconciledRun = useRef<string | null>(null);
  const history = useQuery<ConversationHistory>({
    queryKey: ["messages", id],
    queryFn: () => api("/conversations/" + id + "/messages"),
    enabled: !!id,
  });
  useEffect(() => { if (projectId) setSelectedProject(projectId); }, [projectId]);
  useEffect(() => {
    try {
      sessionStorage.setItem(draftKey, text);
    } catch {
      // Storage can be disabled; the draft simply stays in memory.
    }
  }, [draftKey, text]);
  useEffect(() => {
    if (welcomePicked.current || !workspace) return;
    welcomePicked.current = true;
    setWelcome(
      nextVisit({
        ...workspace,
        projectCount: projects.data?.length ?? 0,
        agentCount: agents.data?.length ?? 0,
      }).welcome,
    );
  }, [agents.data, projects.data, workspace]);
  const run = useQuery<Run>({
    queryKey: ["run", runId],
    queryFn: () => api("/runs/" + runId),
    enabled: !!runId,
    refetchInterval: (q) =>
      !streamConnected && ACTIVE_RUN_STATUSES.includes(q.state.data?.status as RunStatus)
        ? 1500 : false,
  });
  const runActive = ACTIVE_RUN_STATUSES.includes(run.data?.status as RunStatus);
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
    modelInitializedFor.current = null;
    setRunId("");
    setEvents([]);
    resetStreamBuffers();
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
        setAgent(selection.agent_id || "");
        setMode(selection.mode || "chat");
        setResearchMode(Boolean((selection as { research_mode?: boolean }).research_mode));
      }
    }
  }, [history.data, id]);
  useEffect(() => {
    // The composer used to start every chat on "auto", so the model chosen in
    // Settings ▸ 既定モデル was silently ignored. Seed it from the conversation
    // selection, then the app default. Only once per conversation so an explicit
    // pick is never overwritten by a refetch.
    if (!settings.data) return;
    if (id && !history.data) return;
    if (modelInitializedFor.current === (id ?? "")) return;
    modelInitializedFor.current = id ?? "";
    setModel(
      history.data?.selection?.model_id ||
        settings.data.data.default_model_id ||
        "auto",
    );
  }, [history.data, id, settings.data]);
  useEffect(() => {
    if (!runId) return;
    // Opening a finished conversation used to attach an EventSource that
    // replayed every stored token, briefly rendering the last answer a second
    // time. Terminal runs have nothing to stream; the messages query is enough.
    const storedRun = history.data?.runs?.find((r) => r.id === runId);
    if (storedRun && TERMINAL_RUN_STATUSES.includes(storedRun.status as RunStatus)) {
      setStreamConnected(false);
      return;
    }
    setEvents([]);
    resetStreamBuffers();
    const source = new EventSource("/api/v1/runs/" + runId + "/events");
    eventSourceRef.current = source;
    let reconnectErrors = 0;
    source.onopen = () => {
      reconnectErrors = 0;
      setStreamConnected(true);
    };
    // A run is bounded by its budget on the server; if the stream never
    // terminates, bail out and let the polling fallback take over.  The
    // browser reconnects transient SSE failures itself, so this watchdog only
    // covers a stream that stays open but silent for an hour.
    const watchdog = setTimeout(() => {
      if (source.readyState === EventSource.OPEN) {
        source.close();
        setStreamConnected(false);
      }
    }, 60 * 60 * 1000);
    const finish = () => {
      clearTimeout(watchdog);
      source.close();
      flushStreamBuffers();
      setBusy(false);
      setStreamConnected(false);
      qc.invalidateQueries({ queryKey: ["messages", id] });
      qc.invalidateQueries({ queryKey: ["run", runId] });
    };
    const onActivity = (event: Event) => {
      if (!(event instanceof MessageEvent) || typeof event.data !== "string") return;
      const v = parseRunEvent(event.data);
      if (!v) return;
      if (v.kind === "text") {
        streamTextBuffer.current += v.text;
        scheduleStreamFlush();
      } else if (v.kind === "reasoning") {
        reasoningBuffer.current += v.text;
        scheduleStreamFlush();
      } else if (v.kind === "model_started") {
        streamTextBuffer.current = "";
        reasoningBuffer.current = reasoningBuffer.current ? reasoningBuffer.current + "\n\n" : "";
        flushStreamBuffers();
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
          streamTextBuffer.current = "";
          flushStreamBuffers();
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
      // EventSource reconnects with Last-Event-ID by itself.  Keep the stream
      // state false so polling can cover the gap, and only give up after a few
      // consecutive failures (for example an expired session).
      setStreamConnected(false);
      reconnectErrors += 1;
      if (source.readyState === EventSource.CLOSED || reconnectErrors >= 5) {
        clearTimeout(watchdog);
        source.close();
        setBusy(false);
        setError(new Error(ja.streamLost));
        qc.invalidateQueries({ queryKey: ["messages", id] });
        qc.invalidateQueries({ queryKey: ["run", runId] });
      }
    };
    return () => {
      clearTimeout(watchdog);
      source.close();
      if (streamFrame.current !== null) {
        cancelAnimationFrame(streamFrame.current);
        streamFrame.current = null;
      }
      if (eventSourceRef.current === source) eventSourceRef.current = null;
    };
  }, [runId, id, qc]);
  useEffect(() => {
    if (!atBottomRef.current) return;
    const el = messagesRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [streamText, history.data]);
  useEffect(() => {
    // Entering another conversation starts at its newest message.
    const el = messagesRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
    atBottomRef.current = true;
    setAtBottom(true);
  }, [id]);
  const handleMessagesScroll = () => {
    const el = messagesRef.current;
    if (!el) return;
    const next =
      el.scrollHeight - el.scrollTop - el.clientHeight < 80;
    if (next !== atBottomRef.current) {
      atBottomRef.current = next;
      setAtBottom(next);
    }
  };
  const scrollToBottom = () => {
    const el = messagesRef.current;
    if (!el) return;
    el.scrollTo({
      top: el.scrollHeight,
      behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth",
    });
    atBottomRef.current = true;
    setAtBottom(true);
  };
  useEffect(() => {
    if (composer.current) {
      composer.current.style.height = "auto";
      composer.current.style.height = composer.current.scrollHeight + "px";
    }
  }, [text]);
  const active = Boolean(
    run.data && ACTIVE_RUN_STATUSES.includes(run.data.status as RunStatus),
  );
  const live = liveToolCalls(events, runId);
  // Precompute run → message once; the old render did a nested filter/find for
  // every message on every streamed frame (O(n²) on long chats).
  const callsByMessage = useMemo(() => {
    const runToMessage = new Map(
      (history.data?.runs || []).map((run) => [run.id, run.message_id]),
    );
    const byMessage = new Map<string, ToolCallHistory[]>();
    for (const call of toolHistory.data || []) {
      const messageId = runToMessage.get(call.run_id);
      if (!messageId) continue;
      const list = byMessage.get(messageId);
      if (list) list.push(call);
      else byMessage.set(messageId, [call]);
    }
    return byMessage;
  }, [history.data?.runs, toolHistory.data]);
  const currentRunMessageId = history.data?.runs?.find((r) => r.id === runId)?.message_id;
  const latestUserMessageId = [...(history.data?.messages || [])].reverse().find((msg) => msg.data.role === "user")?.id;
  async function send(e?: FormEvent) {
    e?.preventDefault();
    if ((!text.trim() && !attachments.length) || busy || active || uploading) return;
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
        { retryUnsafe: true },
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
    setUploading(true);
    const failed: string[] = [];
    try {
      for (const file of files) {
        if (file.size > 20 * 1024 * 1024) {
          failed.push(`${file.name}: ${t("20MBを超えています")}`);
          continue;
        }
        try {
          const f = new FormData();
          f.set("file", file);
          const a = await api<Artifact>("/artifacts", "POST", f);
          setAttachments((xs) => [...xs, a]);
        } catch (cause) {
          failed.push(`${file.name}: ${cause instanceof Error ? cause.message : String(cause)}`);
        }
      }
      if (failed.length) {
        setError(new Error(failed.join("\n")));
      }
    } finally {
      setUploading(false);
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
  function startEdit(m: Row) {
    setEditingId(m.id);
    setEditText(String(m.data.content || ""));
    setError(null);
  }
  function cancelEdit() {
    setEditingId(null);
    setEditText("");
  }
  /** Replace a sent user turn and regenerate from it. The backend drops that
   *  turn and everything after it, so the reply stream starts clean. */
  async function regenerate(m: Row, contentOverride?: string) {
    const content = (contentOverride ?? editText).trim();
    if (!content || !id || savingEdit || active || busy) return;
    setSavingEdit(true);
    setError(null);
    try {
      const body = {
        content,
        model_id: model,
        mode: researchMode ? "agent" : mode,
        agent_id: agent,
        artifact_ids: (m.data.artifact_ids as string[]) || [],
        temporary_mode: temporaryMode,
        allow_tools: allowTools,
        research_mode: researchMode && !temporaryMode,
      };
      const r = await api<SendMessage>(
        "/conversations/" + id + "/messages/" + m.id + "/edit",
        "POST",
        body,
        uuid(),
        { retryUnsafe: true },
      );
      cancelEdit();
      setRunId(r.run_id);
      qc.invalidateQueries({ queryKey: ["messages", id] });
      qc.invalidateQueries({ queryKey: ["tool-calls", id] });
      qc.invalidateQueries({ queryKey: ["/conversations"] });
    } catch (e) {
      setError(e);
    } finally {
      setSavingEdit(false);
    }
  }
  const saveEdit = (m: Row) => regenerate(m);
  const lastUserMessage = [...(history.data?.messages || [])]
    .reverse()
    .find((m) => m.data.role === "user");
  async function retryLastRun() {
    if (!lastUserMessage) return;
    await regenerate(lastUserMessage, String(lastUserMessage.data.content || ""));
  }
  function retryRequest() {
    setError(null);
    if (runId) {
      setRunId("");
      setTimeout(() => setRunId(runId), 10);
    }
    void history.refetch();
    void models.refetch();
    void tools.refetch();
    void permissionRules.refetch();
    void settings.refetch();
  }
  const conversationTitle = (() => {
    const content = String(lastUserMessage?.data.content || "").trim();
    if (!content) return t("チャット");
    return content.length > 60 ? content.slice(0, 60) + "…" : content;
  })();
  useEffect(() => {
    if (!id) return;
    document.title = `${conversationTitle} · MIX agent`;
  }, [conversationTitle, id]);
  return (
    <div className="chat-page">
      {id && (
        <header className="chat-heading">
          <h1 title={conversationTitle}>{conversationTitle}</h1>
        </header>
      )}
      <div className="chat-body">
        <div className="conversation">
          {id && history.isLoading ? (
            <div className="messages message-skeleton-list" aria-busy="true" aria-label={t("読み込み中…")}>
              {[0, 1, 2].map((i) => (
                <div className="message-skeleton" key={i}>
                  <div className="skeleton skeleton-title" />
                  <div className="skeleton skeleton-text" />
                  <div className="skeleton skeleton-text short" />
                </div>
              ))}
            </div>
          ) : !id && !history.data?.messages?.length && welcome ? (
            <section className="welcome">
              <div className="welcome-icon">
                <Sparkles size={30} />
              </div>
              <p className="eyebrow">MIX</p>
              <h1>{welcomeTitle(welcome)}</h1>
              <div className="welcome-note">
                <ShieldCheck size={14} /> {welcomeNote(welcome)}
              </div>
              <div className="suggestions">
                {welcome.suggestions.map((suggestion) => (
                  <button
                    key={suggestion.id}
                    onClick={() => {
                      setText(greetingText(suggestionPromptId(suggestion.id), suggestion.prompt));
                      composer.current?.focus();
                    }}
                  >
                    <b>{greetingText(suggestionLabelId(suggestion.id), suggestion.label)}</b>
                    <CornerDownLeft size={15} />
                  </button>
                ))}
              </div>
            </section>
          ) : (
            <div className="messages-wrap">
            <div className="messages" ref={messagesRef} onScroll={handleMessagesScroll}>
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
                  {editingId === m.id ? (
                    <div className="message-edit">
                      <textarea
                        aria-label={t("メッセージを編集")}
                        value={editText}
                        autoFocus
                        onChange={(e) => setEditText(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Escape") {
                            e.preventDefault();
                            cancelEdit();
                          }
                          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                            e.preventDefault();
                            void saveEdit(m);
                          }
                        }}
                      />
                      <div className="message-edit-actions">
                        <span className="message-edit-hint">{t("⌘+Enter で保存")}</span>
                        <Button type="button" variant="ghost" onClick={cancelEdit} disabled={savingEdit}>
                          {t("キャンセル")}
                        </Button>
                        <Button
                          type="button"
                          onClick={() => void saveEdit(m)}
                          disabled={savingEdit || !editText.trim()}
                        >
                          {t("保存")}
                        </Button>
                      </div>
                    </div>
                  ) : (
                    <div className="markdown">
                      <Markdown
                        remarkPlugins={[remarkGfm, remarkBreaks]}
                        components={{
                          a: ({ node: _node, ...props }) => (
                            <a {...props} target="_blank" rel="noopener noreferrer" />
                          ),
                        }}
                      >
                        {m.data.content}
                      </Markdown>
                    </div>
                  )}
                  {m.data.role === "user" && editingId !== m.id && (
                    <button
                      type="button"
                      className="message-edit-trigger"
                      aria-label={t("メッセージを編集")}
                      title={t("メッセージを編集")}
                      disabled={active || busy}
                      onClick={() => startEdit(m)}
                    >
                      <Pencil size={12} /> {t("編集")}
                    </button>
                  )}
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
                    const calls = callsByMessage.get(m.id) || [];
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
                {m.data.role === "user" && (currentRunMessageId ? currentRunMessageId === m.id : latestUserMessageId === m.id) && (
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
                    <Markdown
                      remarkPlugins={[remarkGfm, remarkBreaks]}
                      components={{
                        a: ({ node: _node, ...props }) => (
                          <a {...props} target="_blank" rel="noopener noreferrer" />
                        ),
                      }}
                    >
                      {streamText}
                    </Markdown>
                  </div>
                </article>
              )}
              {active && !streamText && (
                <p className="processing" role="status">
                  <span className="pulse" />
                  {t(run.data?.status === "paused" ? "ブラウザを手動操作中" : run.data?.status === "waiting_approval"
                    ? "操作の承認を待っています"
                    : "処理しています…")}
                </p>
              )}
              {run.data?.reason && (
                <div className="notice">
                  <span>{run.data.reason}</span>
                  {(run.data.status === "failed" || run.data.status === "cancelled") && lastUserMessage && (
                    <span className="notice-actions">
                      <Button type="button" variant="outline" onClick={() => void retryLastRun()} disabled={active || savingEdit}>
                        {t("再試行")}
                      </Button>
                    </span>
                  )}
                </div>
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
            </div>
            {!atBottom && (
              <button
                type="button"
                className="scroll-to-bottom"
                onClick={scrollToBottom}
                aria-label={t("最新のメッセージへ")}
                title={t("最新のメッセージへ")}
              >
                <ArrowDown size={15} />
                <span>{t("最新へ")}</span>
              </button>
            )}
            </div>
          )}
          <div className="composer-wrap">
            <ErrorBox
              error={error || history.error || models.error || tools.error || permissionRules.error || settings.error}
              onRetry={retryRequest}
            />
            <form className="composer" onSubmit={send}>
              {temporaryMode && <p className="temporary-notice" role="status">{t("一時モード: Run終了後にMIX側の会話・回答・送信した添付を削除します。選択した添付は送信前にアップロードされます。Toolは")}{t(allowTools ? "許可中（外部への送信や副作用が残る場合があります）" : "無効")}。</p>}
              {attachments.length > 0 && (
                <div className="attachments">
                  {attachments.map((a) => (
                    <span key={a.artifact_id} className={a.mime?.startsWith("image/") ? "attachment-image-span" : ""}>
                      {a.mime?.startsWith("image/") ? (
                        <img src={`/api/v1/artifacts/${a.artifact_id}`} alt="" className="attachment-thumb" />
                      ) : (
                        <FileText size={13} />
                      )}
                      <span className="attachment-name">{a.name}</span>
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
                ref={composer}
                aria-label={t("メッセージ")}
                placeholder={t("MIX agent にメッセージを送信…")}
                value={text}
                onChange={(e) => setText(e.target.value)}
                onPaste={(e) => {
                  const items = Array.from(e.clipboardData.items);
                  const files = items
                    .filter((item) => item.kind === "file")
                    .map((item) => item.getAsFile())
                    .filter((f): f is File => f !== null);
                  if (files.length > 0) {
                    e.preventDefault();
                    void attach(files);
                  }
                }}
                onKeyDown={(e) => {
                  if (
                    e.key === "Enter" &&
                    !e.shiftKey &&
                    !e.nativeEvent.isComposing && e.nativeEvent.keyCode !== 229
                  ) {
                    e.preventDefault();
                    void send();
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
                  {uploading && <span className="send-hint" role="status">{t("アップロード中…")}</span>}
                  <span className="send-hint">{t("Enterで送信 / Shift+Enterで改行")}</span>
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
                        busy || uploading || (!text.trim() && !attachments.length)
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
