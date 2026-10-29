import { api, type Row } from "@/app/api";
import { t } from "@/app/i18n";
import { useRows } from "@/components/shared";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";

import { ChatComposer } from "./ChatComposer";
import { ChatTranscript } from "./ChatTranscript";
import { ChatWelcome } from "./ChatWelcome";
import { nextVisit, type Welcome, type WorkspaceSnapshot } from "./greeting";
import {
  liveToolCalls,
  ACTIVE_RUN_STATUSES,
  type ApprovalDecision,
  type Artifact,
  type ChatAgent,
  type ChatMode,
  type ChatModel,
  type ConversationHistory,
  type PermissionRule,
  type Run,
  type RunStatus,
  type Tool,
  type ToolCallHistory,
} from "./types";
import { useChatActions } from "./useChatActions";
import { useChatRunStream } from "./useChatRunStream";

export function Chat({ workspace }: { workspace: WorkspaceSnapshot | null }) {
  const { id, projectId } = useParams();
  const draftKey = `mix:web-draft:${id ?? "new"}`;
  const navigate = useNavigate();
  const qc = useQueryClient();
  const models = useRows<ChatModel>("/models"),
    providers = useRows<Row>("/providers"),
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
    [sendFailed, setSendFailed] = useState(false),
    [busy, setBusy] = useState(false),
    [attachments, setAttachments] = useState<Artifact[]>([]),
    [uploading, setUploading] = useState(false),
    [runId, setRunId] = useState("");
  // Inline edit of a sent user turn, and the regeneration it triggers.
  const [editingId, setEditingId] = useState<string | null>(null),
    [editText, setEditText] = useState(""),
    [savingEdit, setSavingEdit] = useState(false);
  const [welcome, setWelcome] = useState<Welcome | null>(null);
  // `nextVisit` advances the stored visit count, so guard against React
  // StrictMode's double-invoked effect: without this the count (and the recent
  // fragment list) would be bumped twice per mount.
  const welcomePicked = useRef(false);
  const composer = useRef<HTMLTextAreaElement>(null);
  const restoredConversation = useRef<string | undefined>(undefined);
  const modelInitializedFor = useRef<string | null>(null);
  const reconciledRun = useRef<string | null>(null);
  const history = useQuery<ConversationHistory>({
    queryKey: ["messages", id],
    queryFn: () => api("/conversations/" + id + "/messages"),
    enabled: !!id,
  });
  const { streamText, reasoning, events, streamConnected } = useChatRunStream({
    conversationId: id,
    runId,
    storedRuns: history.data?.runs,
    onFinish: () => setBusy(false),
    onStreamLost: setError,
  });
  const run = useQuery<Run>({
    queryKey: ["run", runId],
    queryFn: () => api("/runs/" + runId),
    enabled: !!runId,
    refetchInterval: (q) =>
      !streamConnected &&
      ACTIVE_RUN_STATUSES.includes(q.state.data?.status as RunStatus)
        ? 1500
        : false,
  });
  const runActive = ACTIVE_RUN_STATUSES.includes(run.data?.status as RunStatus);
  const toolHistory = useQuery<ToolCallHistory[]>({
    queryKey: ["tool-calls", id],
    queryFn: () => api("/conversations/" + id + "/tool-calls"),
    enabled: !!id,
    refetchInterval: !streamConnected && runActive ? 5000 : false,
  });
  useEffect(() => {
    if (projectId) setSelectedProject(projectId);
  }, [projectId]);
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
  useEffect(() => {
    if (!runId || !run.data || streamConnected) return;
    if (
      !["completed", "failed", "cancelled", "interrupted"].includes(
        run.data.status,
      )
    )
      return;
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
  }, [id]);
  useEffect(() => {
    const latestRun = history.data?.runs?.at(-1);
    if (latestRun) setRunId(latestRun.id);
    if (id && history.data && restoredConversation.current !== id) {
      restoredConversation.current = id;
      setSelectedProject(
        (history.data as ConversationHistory & { project_id?: string })
          .project_id || "",
      );
      const selection = history.data.selection;
      if (selection) {
        setAgent(selection.agent_id || "");
        setMode(selection.mode || "chat");
        setResearchMode(
          Boolean((selection as { research_mode?: boolean }).research_mode),
        );
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
  const currentRunMessageId = history.data?.runs?.find(
    (r) => r.id === runId,
  )?.message_id;
  const latestUserMessageId = [...(history.data?.messages || [])]
    .reverse()
    .find((msg) => msg.data.role === "user")?.id;
  const hasNoProviders =
    !providers.isLoading &&
    !providers.isError &&
    (providers.data?.length ?? 0) === 0;
  const hasNoModels =
    !models.isLoading && !models.isError && (models.data?.length ?? 0) === 0;
  const needsProviderSetup = hasNoProviders || hasNoModels;
  const lastUserMessage = [...(history.data?.messages || [])]
    .reverse()
    .find((m) => m.data.role === "user");
  const { send, attach, rate, startEdit, cancelEdit, saveEdit, retryLastRun } =
    useChatActions({
      conversationId: id,
      navigate,
      queryClient: qc,
      text,
      attachments,
      busy,
      active,
      uploading,
      needsProviderSetup,
      hasNoProviders,
      model,
      mode,
      researchMode,
      agent,
      temporaryMode,
      allowTools,
      selectedProject,
      editText,
      savingEdit,
      lastUserMessage,
      setText,
      setAttachments,
      setBusy,
      setError,
      setSendFailed,
      setRunId,
      setUploading,
      setEditingId,
      setEditText,
      setSavingEdit,
    });
  function retryRequest() {
    setError(null);
    if (runId) {
      setRunId("");
      setTimeout(() => setRunId(runId), 10);
    }
    void history.refetch();
    void models.refetch();
    void providers.refetch();
    void tools.refetch();
    void permissionRules.refetch();
    void settings.refetch();
  }
  function handleApproval(
    approvalId: string,
    decision: ApprovalDecision,
  ): Promise<void> {
    return api("/approvals/" + approvalId + "/decision", "POST", {
      decision,
    })
      .then(() => {
        qc.invalidateQueries({ queryKey: ["tool-calls", id] });
        qc.invalidateQueries({ queryKey: ["run", runId] });
      })
      .catch((e: unknown) => setError(e));
  }
  function handleBrowserChange(browserRunId: string) {
    qc.invalidateQueries({ queryKey: ["run", runId] });
    qc.invalidateQueries({ queryKey: ["browser-frames", browserRunId] });
  }
  function handleAgentChange(nextAgent: string) {
    setAgent(nextAgent);
    const next = agents.data?.find((x) => x.id === nextAgent);
    if (next) {
      setMode(next.data.mode);
      if (next.data.mode !== "agent") setResearchMode(false);
      if (next.data.model_id) setModel(next.data.model_id);
    }
  }
  function handleModeChange(next: ChatMode) {
    setMode(next);
    if (next !== "agent") setResearchMode(false);
  }
  function handleResearchModeChange(checked: boolean) {
    setResearchMode(checked);
    if (checked) setMode("agent");
  }
  function handleTemporaryModeChange(value: boolean) {
    setTemporaryMode(value);
    if (value) setResearchMode(false);
  }
  function stopRun() {
    void api("/runs/" + runId + "/cancel", "POST").catch((e: unknown) =>
      setError(e),
    );
  }
  function handleResumed() {
    qc.invalidateQueries({ queryKey: ["run", runId] });
    qc.invalidateQueries({ queryKey: ["tool-calls", id] });
    setRunId("");
    setTimeout(() => setRunId(runId), 10);
  }
  function handleBudgetResolved() {
    qc.invalidateQueries({ queryKey: ["run", runId] });
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
  function pickSuggestion(prompt: string) {
    setText(prompt);
    composer.current?.focus();
  }
  const showSkeleton = !!id && history.isLoading;
  const showWelcome =
    !id && !history.data?.messages?.length && Boolean(welcome);
  return (
    <div className="chat-page">
      {id && (
        <header className="chat-heading">
          <h1 title={conversationTitle}>{conversationTitle}</h1>
        </header>
      )}
      <div className="chat-body">
        <div className="conversation">
          {showSkeleton || showWelcome ? (
            <ChatWelcome
              loading={showSkeleton}
              welcome={showWelcome ? welcome : null}
              onPickSuggestion={pickSuggestion}
            />
          ) : (
            <ChatTranscript
              conversationId={id}
              messages={history.data?.messages}
              runs={history.data?.runs}
              todos={history.data?.todos}
              allToolCalls={toolHistory.data}
              callsByMessage={callsByMessage}
              liveCalls={live}
              currentRunMessageId={currentRunMessageId}
              latestUserMessageId={latestUserMessageId}
              active={active}
              runId={runId}
              run={run.data}
              streamText={streamText}
              reasoning={reasoning}
              fetchedAt={run.dataUpdatedAt}
              editingId={editingId}
              editText={editText}
              savingEdit={savingEdit}
              editDisabled={active || busy}
              canRetry={!!lastUserMessage}
              onEditChange={setEditText}
              onEditCancel={cancelEdit}
              onEditSave={(message) => void saveEdit(message)}
              onEditStart={startEdit}
              onRate={rate}
              onApproval={handleApproval}
              onBrowserChange={handleBrowserChange}
              onRetryRun={() => void retryLastRun()}
              onError={setError}
              onResumed={handleResumed}
              onBudgetResolved={handleBudgetResolved}
            />
          )}
          <ChatComposer
            text={text}
            onTextChange={setText}
            onSubmit={send}
            textareaRef={composer}
            attachments={attachments}
            onRemoveAttachment={(attachment) =>
              setAttachments((xs) => xs.filter((x) => x !== attachment))
            }
            onAttach={attach}
            uploading={uploading}
            models={models.data}
            agents={agents.data}
            projects={projects.data}
            tools={tools.data}
            permissionRules={permissionRules.data}
            model={model}
            onModelChange={setModel}
            mode={mode}
            onModeChange={handleModeChange}
            researchMode={researchMode}
            onResearchModeChange={handleResearchModeChange}
            agent={agent}
            onAgentChange={handleAgentChange}
            selectedProject={selectedProject}
            onSelectedProjectChange={setSelectedProject}
            conversationId={id}
            temporaryMode={temporaryMode}
            onTemporaryModeChange={handleTemporaryModeChange}
            allowTools={allowTools}
            onAllowToolsChange={setAllowTools}
            active={active}
            busy={busy}
            needsProviderSetup={needsProviderSetup}
            hasNoProviders={hasNoProviders}
            pageError={
              error ||
              history.error ||
              models.error ||
              providers.error ||
              tools.error ||
              permissionRules.error ||
              settings.error
            }
            onRetry={sendFailed ? () => void send() : retryRequest}
            retryLabel={sendFailed ? t("再送信") : undefined}
            onConnectProvider={() => navigate("/settings/providers")}
            onStop={stopRun}
            run={run.data}
          />
        </div>
      </div>
    </div>
  );
}
