import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { useRunEvents } from "@/lib/runStore";
import { isBusy, isTerminal } from "@/lib/runEvents";
import {
  CONVERSATIONS_QUERY_KEY, useConversations, useModels, useProjects,
} from "@/lib/queries";
import { useUi } from "@/lib/ui";
import { getLastModelForSurface } from "@/lib/modelPrefs";
import type { ModelView, RunView } from "@/lib/types";
import { nextVisit, type Welcome } from "./greeting";
import { ChatComposerProvider, useChatComposerContext } from "./ChatComposerContext";
import { Composer, type DraftAttachment } from "./Composer";
import { Transcript } from "./Transcript";
import { ErrorBox } from "@/components/shared";
import { PaneErrorBoundary } from "@/components/PaneErrorBoundary";

/** Stable empties for the memoized `Transcript` props. */
const EMPTY_MODELS: ModelView[] = [];

export function ChatPane() {
  return (
    <ChatComposerProvider>
      <ChatPaneBody />
    </ChatComposerProvider>
  );
}

function ChatPaneBody() {
  // Per-field selectors: subscribing to the whole store re-rendered the
  // transcript and composer on every unrelated UI change.
  const conversationId = useUi((s) => s.conversationId);
  const setConversationId = useUi((s) => s.setConversationId);
  const mode = useUi((s) => s.mode);
  const modelId = useUi((s) => s.modelId);
  const setModelId = useUi((s) => s.setModelId);
  const activeProjectId = useUi((s) => s.activeProjectId);
  const setActiveProjectId = useUi((s) => s.setActiveProjectId);
  const setWorkspaceId = useUi((s) => s.setWorkspaceId);
  const surface = useUi((s) => s.surface);
  const projectError = useChatComposerContext()?.projectError ?? null;
  const queryClient = useQueryClient();
  const liveRunId = useRunEvents((s) => s.liveRunId);
  const liveConversationId = useRunEvents((s) => s.conversationId);
  const liveTimeline = useRunEvents((s) => s.timeline);
  const startRun = useRunEvents((s) => s.startRun);
  const clearLive = useRunEvents((s) => s.clear);

  const conversations = useConversations();
  const models = useModels();
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

  // A temporary send that created its own conversation: once the run ends the
  // backend deletes that thread, so the composer has to let go of it too.
  const temporaryNewRef = useRef<string | null>(null);

  const activeModelId = modelId || "auto";
  const activeConversationId = conversationId ?? conversations.data?.[0]?.id ?? null;

  useEffect(() => {
    // `auto` is the default pick and is valid without any model in the list.
    // Adopt the last model used on this surface when it is still available
    // (or `auto`), and otherwise fall back to `auto` rather than the first
    // model. A model that has disappeared from the list (typically because its
    // provider was deleted) is replaced too: left in place it would keep being
    // sent with every new message, producing a run against a model id no
    // provider can serve. `models.data` is undefined while loading, which is
    // not the same as "the list is empty", so the reset waits for the query
    // to settle.
    if (models.data === undefined) return;
    if (!modelId || (modelId !== "auto" && !models.data.some((m) => m.id === modelId))) {
      const remembered = getLastModelForSurface(surface);
      const next =
        (remembered === "auto" ? "auto" : models.data.find((m) => m.id === remembered)?.id) ??
        "auto";
      if (next !== modelId) setModelId(next);
    }
  }, [modelId, models.data, setModelId, surface]);

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
  }, [activeConversation?.project_id, activeProjectId, setActiveProjectId]);

  // Keep the ui-store workspace in step with the active conversation so the
  // Browser pane defaults to the same profile the agent uses.
  useEffect(() => {
    setWorkspaceId(activeConversation?.workspace_id ?? null);
  }, [activeConversation?.workspace_id, setWorkspaceId]);

  const sendMessage = useMutation({
    mutationFn: async ({
      content,
      attachments,
      temporaryMode,
      allowTools,
    }: {
      content: string;
      attachments: DraftAttachment[];
      temporaryMode: boolean;
      allowTools: boolean;
    }) => {
      let target = activeConversationId;
      // Whether this send created its conversation: a temporary turn in a brand
      // new thread takes the whole thread with it when it is purged.
      const createdConversation = !target;
      if (!target) {
        const created = await api.createConversation({
          mode, model_id: activeModelId, title: content.slice(0, 40), workspace_id: null,
          project_id: activeProjectId,
        });
        setConversationId(created.id);
        target = created.id;
      }
      const out = await api.sendMessage({
        conversation_id: target,
        content,
        model_id: activeModelId,
        mode,
        workspace_id: null,
        attachments: attachments.map((a) => ({ ...a, filename: a.filename })),
        reasoning: null,
        temperature: null,
        max_tokens: null,
        temporary_mode: temporaryMode,
        allow_tools: allowTools,
      });
      // Return the conversation the run actually belongs to. When this turn
      // created it, `activeConversationId` in the closure below is still null,
      // so the run used to be tagged to the wrong conversation and its messages
      // were invalidated against a null key.
      return {
        runId: out.run_id,
        conversationId: target,
        purged: temporaryMode && createdConversation,
      };
    },
    onSuccess: ({ runId, conversationId, purged }) => {
      if (purged) temporaryNewRef.current = conversationId;
      startRun(runId, conversationId);
      void queryClient.invalidateQueries({ queryKey: ["messages", conversationId] });
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
    },
  });

  const cancel = useMutation({
    mutationFn: (id: string) => api.cancelRun(id),
  });

  // Edit a user turn and regenerate from it. The backend removes that turn and
  // everything after it, then starts a fresh Run, so this mirrors `sendMessage`
  // exactly: adopt the new run and refresh the transcript.
  const editMessage = useMutation({
    mutationFn: async ({ messageId, content }: { messageId: string; content: string }) => {
      if (!activeConversationId) throw new Error("no conversation");
      return api.editMessage({
        conversation_id: activeConversationId,
        message_id: messageId,
        content,
        model_id: activeModelId,
        mode,
        reasoning: null,
      });
    },
    onSuccess: (out) => {
      startRun(out.run_id, activeConversationId);
      void queryClient.invalidateQueries({ queryKey: ["messages", activeConversationId] });
      void queryClient.invalidateQueries({ queryKey: ["runs", activeConversationId] });
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
    },
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

  // A temporary run in a brand-new thread deletes that thread. Wait until the
  // refreshed list no longer carries it before letting go, so the clear cannot
  // race the refetch and leave the composer pointing at a deleted id (or, worse,
  // adopting the wrong thread from the stale list).
  useEffect(() => {
    if (conversations.data === undefined) return;
    const purged = temporaryNewRef.current;
    if (purged && !conversations.data.some((c) => c.id === purged)) {
      temporaryNewRef.current = null;
      setConversationId(conversationId === purged ? null : conversationId);
    }
  }, [conversations.data, conversationId, setConversationId]);

  const runById = useMemo(() => {
    const map = new Map<string, RunView>();
    for (const run of runs.data ?? []) map.set(run.id, run);
    return map;
  }, [runs.data]);

  // Defense in depth: `tool` rows hold raw JSON results for the model's next
  // turn. The backend already excludes them, but an older DB row (or a future
  // role) must never render as an assistant bubble with `{"args":...}`.
  // Empty intermediate assistant turns (tool-call turns with no text) carry
  // no transcript value either — tools surface via the Activity badge.
  const visibleMessages = useMemo(() => {
    const list = messages.data ?? [];
    return list.filter((m) => {
      if (m.role === "user") return true;
      if (m.role !== "assistant") return false;
      const text = (m.content ?? "").trim();
      if (text.length > 0) return true;
      return !!m.reasoning?.trim();
    });
  }, [messages.data]);

  // The live block only renders in the conversation the run belongs to.
  const liveForThisConversation =
    liveTimeline != null &&
    (liveConversationId == null || liveConversationId === activeConversationId)
      ? liveTimeline
      : null;

  const busy = liveForThisConversation != null && isBusy(liveForThisConversation.status);

  // Stable identities for the memoized `Composer`. Inline arrows here meant a
  // fresh prop on every store flush, so the composer — with its model, mode and
  // project menus — re-rendered ~16×/second while a run streamed. Deps use the
  // stable `mutate`/`mutateAsync` functions, not the mutation result objects
  // whose identity changes per render.
  const onSend = useCallback(
    (
      content: string,
      attachments: DraftAttachment[],
      options: { temporaryMode: boolean; allowTools: boolean },
    ) => sendMessage.mutate({ content, attachments, ...options }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [sendMessage.mutate],
  );
  const onEditMessage = useCallback(
    (messageId: string, content: string) =>
      editMessage.mutateAsync({ messageId, content }).then(() => undefined),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [editMessage.mutateAsync],
  );
  const onStop = useCallback(() => {
    if (liveRunId) cancel.mutate(liveRunId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [liveRunId, cancel.mutate]);
  // Retry a failed run by regenerating from the last user turn. The backend
  // removes that turn and everything after it, so re-editing with identical
  // content is exactly a retry — no new prop drilling into MessageItem needed
  // beyond the callback itself.
  const onRetry = useCallback(async () => {
    const list = visibleMessages;
    for (let i = list.length - 1; i >= 0; i -= 1) {
      const m = list[i];
      if (m.role === "user") {
        await editMessage.mutateAsync({ messageId: m.id, content: m.content });
        return;
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visibleMessages, editMessage.mutateAsync]);

  // The empty-state greeting is picked once per visit, from the workspace
  // snapshot. While the thread list is still in flight the Transcript falls
  // back to the brand line, so the greeting never re-picks mid-session.
  const [welcome, setWelcome] = useState<Welcome | null>(null);
  // `nextVisit` advances the stored visit count, so guard against React
  // StrictMode's double-invoked effect: without this the count (and the recent
  // fragment list) would be bumped twice per mount.
  const welcomePicked = useRef(false);
  useEffect(() => {
    if (welcomePicked.current || conversations.data === undefined) return;
    welcomePicked.current = true;
    const rows = conversations.data;
    const stamps = rows
      .map((row) => Date.parse(row.updated_at))
      .filter((stamp) => Number.isFinite(stamp));
    setWelcome(
      nextVisit({
        conversationCount: rows.length,
        messageCount: rows.reduce((sum, row) => sum + (row.message_count || 0), 0),
        projectCount: projects.data?.length ?? 0,
        lastActivityAt: stamps.length ? Math.max(...stamps) : null,
        surface,
        projectActive: !!activeProjectId,
      }).welcome,
    );
  }, [activeProjectId, conversations.data, projects.data, surface]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PaneErrorBoundary name="Transcript" resetKeys={[activeConversationId]}>
      <Transcript
        messages={visibleMessages}
        runs={runById}
        models={models.data ?? EMPTY_MODELS}
        live={liveForThisConversation}
        liveRunId={liveForThisConversation ? liveRunId : null}
        loading={messages.isPending}
        error={messages.isError ? messages.error : null}
        welcome={welcome}
        onEditMessage={onEditMessage}
        onRetry={onRetry}
        editDisabled={busy}
        retryDisabled={busy || editMessage.isPending}
      />
      </PaneErrorBoundary>
      {(sendMessage.isError ||
        editMessage.isError ||
        projectError ||
        cancel.isError) && (
        <div className="mx-auto w-full max-w-content shrink-0 px-4 pb-1 sm:px-6">
          <ErrorBox
            error={
              sendMessage.error ?? editMessage.error ?? cancel.error ?? projectError
            }
            action={
              sendMessage.isError || editMessage.isError ? t("再試行") : undefined
            }
            onAction={
              sendMessage.isError || editMessage.isError
                ? () => {
                    // Same guard as the old disabled attribute: a retry must not
                    // race a run that is already streaming or a pending re-edit.
                    if (busy || editMessage.isPending) return;
                    void onRetry();
                  }
                : undefined
            }
          />
        </div>
      )}
      <div className="shrink-0">
      {/* Model/mode/project state comes from `ChatComposerProvider` above —
          only the run mutations stay drilled. */}
      <PaneErrorBoundary name="Composer" resetKeys={[activeConversationId]}>
      <Composer
        busy={busy}
        onSend={onSend}
        onStop={onStop}
        draftKey={activeConversationId ?? "none"}
      />
      </PaneErrorBoundary>
      </div>
    </div>
  );
}
