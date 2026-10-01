import { useCallback, useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { useRunEvents } from "@/lib/runStore";
import { isBusy, isTerminal } from "@/lib/runEvents";
import {
  CONVERSATIONS_QUERY_KEY, PROJECTS_QUERY_KEY, useConversations, useProjects,
} from "@/lib/queries";
import { SURFACE_MODES, useUi } from "@/lib/ui";
import type { ModelView, ProjectView, RunView } from "@/lib/types";
import { Composer, type DraftAttachment } from "./Composer";
import { Transcript } from "./Transcript";
import { Alert } from "@/components/ui/alert";

/** Stable empties for the memoized `Composer` props. */
const EMPTY_MODELS: ModelView[] = [];
const EMPTY_PROJECTS: ProjectView[] = [];

export function ChatPane() {
  const [projectError, setProjectError] = useState<string | null>(null);
  // Per-field selectors: subscribing to the whole store re-rendered the
  // transcript and composer on every unrelated UI change.
  const conversationId = useUi((s) => s.conversationId);
  const setConversationId = useUi((s) => s.setConversationId);
  const mode = useUi((s) => s.mode);
  const setMode = useUi((s) => s.setMode);
  const modelId = useUi((s) => s.modelId);
  const setModelId = useUi((s) => s.setModelId);
  const activeProjectId = useUi((s) => s.activeProjectId);
  const setActiveProjectId = useUi((s) => s.setActiveProjectId);
  const surface = useUi((s) => s.surface);
  const allowedModes = SURFACE_MODES[surface];
  const queryClient = useQueryClient();
  const liveRunId = useRunEvents((s) => s.liveRunId);
  const liveConversationId = useRunEvents((s) => s.conversationId);
  const liveTimeline = useRunEvents((s) => s.timeline);
  const startRun = useRunEvents((s) => s.startRun);
  const clearLive = useRunEvents((s) => s.clear);

  const conversations = useConversations();
  const models = useQuery({ queryKey: ["models"], queryFn: api.listModels });
  const projects = useProjects();
  const messages = useQuery({
    queryKey: ["messages", conversationId],
    queryFn: () => api.listMessages(conversationId!),
    enabled: !!conversationId,
  });
  const runs = useQuery({
    queryKey: ["runs", conversationId],
    queryFn: () => api.listRuns(conversationId!),
    enabled: !!conversationId,
  });

  const activeModelId = modelId || models.data?.[0]?.id || "";
  const activeConversationId = conversationId ?? conversations.data?.[0]?.id ?? null;

  const selectProject = async (projectId: string | null) => {
    if (projectId === activeProjectId) return;
    setProjectError(null);
    setActiveProjectId(projectId);
    try {
      const created = await api.createConversation({
        mode,
        model_id: activeModelId,
        title: null,
        workspace_id: null,
        project_id: projectId,
      });
      setConversationId(created.id);
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
    } catch (error) {
      setActiveProjectId(activeProjectId);
      setProjectError(errorMessage(error));
    }
  };

  const createProject = async (name: string) => {
    // Folder-first: every project is rooted at a directory, so pick one before
    // creating. Cancelling the picker aborts the creation and returns null.
    const path = await openDialog({ directory: true, multiple: false });
    if (typeof path !== "string" || !path) return null;
    const created = await api.createProject({
      name: name.trim(),
      instructions: "",
      rootPath: path,
      sources: [],
    });
    void queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
    return created.id;
  };

  useEffect(() => {
    // Adopt the first model when none is chosen, and *replace* one that has
    // disappeared from the list — typically because its provider was deleted.
    // Left in place it would keep being sent with every new message, producing a
    // run against a model id no provider can serve. `models.data` is undefined
    // while loading, which is not the same as "the list is empty", so the reset
    // waits for the query to settle.
    if (models.data === undefined) return;
    if (!modelId || !models.data.some((m) => m.id === modelId)) {
      if (models.data[0]) setModelId(models.data[0].id);
    }
  }, [modelId, models.data, setModelId]);

  useEffect(() => {
    if (!conversationId && conversations.data?.[0]) setConversationId(conversations.data[0].id);
  }, [conversationId, conversations.data, setConversationId]);

  // Opening a project-scoped thread moves the file tree / diff scope with it.
  const activeConversation = useMemo(
    () => conversations.data?.find((c) => c.id === activeConversationId),
    [conversations.data, activeConversationId],
  );
  useEffect(() => {
    if (activeConversation?.project_id && activeConversation.project_id !== activeProjectId) {
      setActiveProjectId(activeConversation.project_id);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeConversation?.project_id]);

  const sendMessage = useMutation({
    mutationFn: async ({ content, attachments }: { content: string; attachments: DraftAttachment[] }) => {
      let target = activeConversationId;
      if (!target) {
        const created = await api.createConversation({
          mode, model_id: activeModelId, title: content.slice(0, 40), workspace_id: null,
          project_id: activeProjectId,
        });
        setConversationId(created.id);
        target = created.id;
      }
      return api.sendMessage({
        conversation_id: target,
        content,
        model_id: activeModelId,
        mode,
        workspace_id: null,
        attachments: attachments.map((a) => ({ ...a, filename: a.filename })),
        reasoning: null,
        temperature: null,
        max_tokens: null,
      });
    },
    onSuccess: (out) => {
      startRun(out.run_id, activeConversationId);
      void queryClient.invalidateQueries({ queryKey: ["messages", activeConversationId] });
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
    },
  });

  const cancel = useMutation({
    mutationFn: (id: string) => api.cancelRun(id),
  });

  // Terminal status: refresh persisted history, then retire the live view
  // once the assistant message lands (the Transcript hides it either way).
  const status = liveTimeline?.status;
  useEffect(() => {
    if (!liveRunId || !status || !isTerminal(status)) return;
    const target = liveConversationId ?? activeConversationId;
    void queryClient.invalidateQueries({ queryKey: ["messages", target] });
    void queryClient.invalidateQueries({ queryKey: ["runs", target] });
    void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
    void queryClient.invalidateQueries({ queryKey: ["diagnostics"] });
    const timer = window.setTimeout(() => clearLive(), 1500);
    return () => window.clearTimeout(timer);
  }, [status, liveRunId, liveConversationId, activeConversationId, queryClient, clearLive]);

  const runById = useMemo(() => {
    const map = new Map<string, RunView>();
    for (const run of runs.data ?? []) map.set(run.id, run);
    return map;
  }, [runs.data]);

  // The live block only renders in the conversation the run belongs to.
  const liveForThisConversation =
    liveTimeline != null &&
    (liveConversationId == null || liveConversationId === activeConversationId)
      ? liveTimeline
      : null;

  const busy = liveForThisConversation != null && isBusy(liveForThisConversation.status);

  // Stable identities for the memoized `Composer`. Inline arrows here meant a
  // fresh prop on every store flush, so the composer — with its model, mode and
  // project menus — re-rendered ~16×/second while a run streamed.
  const onSend = useCallback(
    (content: string, attachments: DraftAttachment[]) => sendMessage.mutate({ content, attachments }),
    [sendMessage],
  );
  const onStop = useCallback(() => {
    if (liveRunId) cancel.mutate(liveRunId);
  }, [liveRunId, cancel]);
  const onProjectChange = useCallback(
    (projectId: string | null) => void selectProject(projectId),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [activeProjectId, activeConversationId, activeModelId, mode],
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <Transcript
        messages={messages.data ?? []}
        runs={runById}
        live={liveForThisConversation}
        liveRunId={liveForThisConversation ? liveRunId : null}
        loading={messages.isPending}
        error={messages.isError ? messages.error : null}
      />
      <Composer
        mode={mode}
        setMode={setMode}
        allowedModes={allowedModes}
        models={models.data ?? EMPTY_MODELS}
        modelId={activeModelId}
        setModelId={setModelId}
        projects={projects.data ?? EMPTY_PROJECTS}
        projectId={activeProjectId}
        onProjectChange={onProjectChange}
        onCreateProject={createProject}
        busy={busy}
        onSend={onSend}
        onStop={onStop}
      />
      {(sendMessage.isError || projectError) && (
        <div className="mx-auto w-full max-w-[820px] px-6 pb-3">
          <Alert tone="danger">
            {errorMessage(sendMessage.error ?? projectError)}
          </Alert>
        </div>
      )}
    </div>
  );
}
