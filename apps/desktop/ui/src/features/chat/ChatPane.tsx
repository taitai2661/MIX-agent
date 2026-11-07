import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { useRunEvents } from "@/lib/runStore";
import { isBusy, isTerminal } from "@/lib/runEvents";
import {
  CONVERSATIONS_QUERY_KEY,
  useConversation,
  useConversationStats,
  useLatestConversation,
  useLocalRuntimes,
  useModels,
  useProjects,
  useProviders,
} from "@/lib/queries";
import { useUi, surfaceForMode } from "@/lib/ui";
import { getLastModelForSurface } from "@/lib/modelPrefs";
import { SETTINGS_QUERY_KEY } from "@/lib/settings";
import type { MessageView, ModelView, RunView } from "@/lib/types";
import { nextVisit, type Welcome } from "./greeting";
import {
  ChatComposerProvider,
  useChatComposerContext,
} from "./ChatComposerContext";
import { Composer, type DraftAttachment } from "./Composer";
import { localRuntimeForModel } from "@/lib/localRuntime";
import { recordUxEvent } from "@/lib/uxEvents";
import { RunStatusBar } from "./RunStatusBar";
import { Transcript } from "./Transcript";
import { ApprovalBanner } from "./ApprovalBanner";
import { ErrorBox } from "@/components/shared";
import { PaneErrorBoundary } from "@/components/PaneErrorBoundary";
import { markOnce } from "@/lib/perf";

/** Stable empties for the memoized `Transcript` props. */
const EMPTY_MODELS: ModelView[] = [];
/** Matches the backend's default page size; `hasOlder` is inferred from a full page. */
const MESSAGE_PAGE_SIZE = 100;

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
  const startNewThread = useUi((s) => s.startNewThread);
  const newThreadDraft = useUi((s) => s.newThreadDraft);
  const mode = useUi((s) => s.mode);
  const modelId = useUi((s) => s.modelId);
  const setModelId = useUi((s) => s.setModelId);
  const activeProjectId = useUi((s) => s.activeProjectId);
  const setActiveProjectId = useUi((s) => s.setActiveProjectId);
  const setWorkspaceId = useUi((s) => s.setWorkspaceId);
  const surface = useUi((s) => s.surface);
  const projectError = useChatComposerContext()?.projectError ?? null;
  const queryClient = useQueryClient();
  const startRun = useRunEvents((s) => s.startRun);
  const clearLive = useRunEvents((s) => s.clearRun);

  const conversations = useLatestConversation(
    surface === "desktop-agent" ? ["agent", "plan"] : ["chat", "thinking"],
  );
  const stats = useConversationStats();
  const models = useModels();
  const projects = useProjects();
  const providers = useProviders();
  const localRuntimes = useLocalRuntimes();
  const settings = useQuery({
    queryKey: SETTINGS_QUERY_KEY,
    queryFn: api.getSettings,
  });
  // A temporary send that created its own conversation: once the run ends the
  // backend deletes that thread, so the composer has to let go of it too.
  const [temporaryNewId, setTemporaryNewId] = useState<string | null>(null);
  // A failed first send deletes the thread it just created. Retrying through
  // `activeConversationId` would then fall back to the newest *existing*
  // thread and deliver the message into the wrong conversation.
  const retryNeedsNewConversation = useRef(false);
  // Counters must not be inflated by effect re-runs (StrictMode, cache
  // invalidations while a run settles) or by a re-render on every token.
  const recordedCompletions = useRef(new Set<string>());
  const recordedApprovals = useRef(new Set<string>());

  const activeModelId = modelId || "auto";
  // A new chat stays blank until its first send, even though the surface has
  // older threads. Falling back to the newest thread here made the composer
  // show (and send into) an existing thread, so the new chat was never created.
  const activeConversationId = newThreadDraft
    ? null
    : (conversationId ?? conversations.data?.id ?? null);
  // Key the history off the conversation the pane actually renders, not the
  // persisted id. On launch `conversationId` is still null for a beat while a
  // thread is being adopted — querying the id made that thread's history look
  // like it was never loading. A brand-new draft has no conversation at all,
  // so the query is disabled and must not report "pending" forever.
  const messages = useQuery({
    queryKey: ["messages", activeConversationId],
    queryFn: () => api.listMessages(activeConversationId!, MESSAGE_PAGE_SIZE),
    enabled: !!activeConversationId,
  });
  // Older pages are kept beside the query result instead of in it, so a live
  // refetch of the newest page never discards history the user loaded.
  useEffect(() => {
    if (!messages.isPending) markOnce("transcript-ready");
  }, [messages.isPending]);
  const [olderMessages, setOlderMessages] = useState<MessageView[]>([]);
  const [hasOlder, setHasOlder] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  useEffect(() => {
    setOlderMessages([]);
    setHasOlder(false);
  }, [activeConversationId]);
  useEffect(() => {
    if (messages.data) setHasOlder(messages.data.length >= MESSAGE_PAGE_SIZE);
  }, [messages.data]);
  const runs = useQuery({
    queryKey: ["runs", activeConversationId],
    queryFn: () => api.listRuns(activeConversationId!),
    enabled: !!activeConversationId,
  });
  // Read this thread's own run identity and status, not its timeline. The
  // timeline changes on every token; subscribing to it here re-rendered the
  // whole pane (composer and transcript included) 16×/s. `LiveBlock` reads the
  // timeline itself, so this component only wakes on a start/status change.
  // A run started in another thread stays tracked (and keeps streaming)
  // instead of being wiped by the next send.
  const liveRunId = useRunEvents((s) =>
    activeConversationId
      ? (s.byConversation[activeConversationId]?.runId ?? null)
      : null,
  );
  const liveStatus = useRunEvents((s) =>
    activeConversationId
      ? (s.byConversation[activeConversationId]?.timeline.status ?? null)
      : null,
  );
  // The approval object is stable between token deltas (the reducer only
  // replaces it when an approval event arrives).
  const liveApproval = useRunEvents((s) =>
    activeConversationId
      ? (s.byConversation[activeConversationId]?.timeline.approval ?? null)
      : null,
  );
  const liveConversationId = liveRunId ? activeConversationId : null;

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
    if (
      !modelId ||
      (modelId !== "auto" && !models.data.some((m) => m.id === modelId))
    ) {
      const remembered = getLastModelForSurface(surface);
      const next =
        (remembered === "auto"
          ? "auto"
          : models.data.find((m) => m.id === remembered)?.id) ?? "auto";
      if (next !== modelId) setModelId(next);
    }
  }, [modelId, models.data, setModelId, surface]);

  const activeConversation = useConversation(activeConversationId);
  // A temporary thread is deleted at the end of its Run. Watch that row so the
  // composer lets go only after the backend has actually removed it.
  const purgedConversation = useConversation(temporaryNewId);
  useEffect(() => {
    // A new chat is deliberately blank until its first send. Without this
    // guard the auto-select below immediately persisted the newest existing
    // thread and the draft's first message landed in that thread.
    if (newThreadDraft) return;
    if (!conversationId) {
      if (conversations.data) setConversationId(conversations.data.id);
      return;
    }
    if (
      activeConversation.data &&
      surfaceForMode(activeConversation.data.mode) !== surface
    ) {
      // The persisted id belongs to the other surface; adopt this surface's
      // newest thread (or clear the id when there is none).
      setConversationId(conversations.data?.id ?? null);
    }
  }, [
    conversationId,
    activeConversation.data,
    conversations.data,
    surface,
    setConversationId,
    newThreadDraft,
  ]);

  // Opening a project-scoped thread moves the file tree / diff scope with it.
  useEffect(() => {
    if (
      activeConversation.data?.project_id &&
      activeConversation.data.project_id !== activeProjectId
    ) {
      setActiveProjectId(activeConversation.data.project_id);
    }
  }, [
    activeConversation.data?.project_id,
    activeProjectId,
    setActiveProjectId,
  ]);

  // Keep the ui-store workspace in step with the active conversation so the
  // Browser pane defaults to the same profile the agent uses.
  useEffect(() => {
    setWorkspaceId(activeConversation.data?.workspace_id ?? null);
  }, [activeConversation.data?.workspace_id, setWorkspaceId]);

  const sendMessage = useMutation({
    mutationFn: async ({
      content,
      attachments,
      temporaryMode,
      allowTools,
      retryNewConversation = false,
    }: {
      content: string;
      attachments: DraftAttachment[];
      temporaryMode: boolean;
      allowTools: boolean;
      /** Retry of a first send whose thread was rolled back: create a new one
       *  instead of falling back to another existing thread. */
      retryNewConversation?: boolean;
    }) => {
      // A stopped local runtime is the most common first-send failure. Starting
      // it *before* the conversation exists means a failed start cannot leave
      // an empty thread behind or discard a just-created id.
      const localSlug = localRuntimeForModel(
        activeModelId,
        models.data ?? [],
        providers.data ?? [],
      );
      if (localSlug) {
        const runtime = localRuntimes.data?.find(
          (entry) => entry.slug === localSlug,
        );
        // Auto-start only when the user opted in, and never after they
        // explicitly stopped the runtime (the Stop button would otherwise be
        // undone by the next message).
        const autoStart = settings.data?.local_auto_start === true;
        if (
          runtime &&
          !runtime.online &&
          runtime.can_start &&
          autoStart &&
          !runtime.auto_start_suppressed
        ) {
          await api.localRuntimeStart(localSlug);
        }
      }
      let target = retryNewConversation ? null : activeConversationId;
      let createdId: string | null = null;
      // Whether this send created its conversation: a temporary turn in a brand
      // new thread takes the whole thread with it when it is purged.
      const createdConversation = !target;
      if (!target) {
        const created = await api.createConversation({
          mode,
          model_id: activeModelId,
          title: content.slice(0, 40),
          workspace_id: null,
          project_id: activeProjectId,
        });
        createdId = created.id;
        // Adopt the new conversation only after the send succeeds. Setting it
        // here flipped the composer's draft slot mid-request, which saved the
        // prompt being sent into the "new chat" slot and cleared attachments
        // that were still uploading. A failed first send also deleted the row,
        // so the pane no longer briefly switched to a thread that vanished.
        target = created.id;
      }
      try {
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
          created: createdConversation,
          purged: temporaryMode && createdConversation,
        };
      } catch (err) {
        // A failed first send used to leave an empty thread behind (and the
        // composer empty). Drop the just-created thread so the draft can be
        // retried without cluttering history.
        retryNeedsNewConversation.current = createdId !== null;
        if (createdId) {
          // Return to a blank draft, not to "no selection": clearing the id
          // alone ends draft mode and the auto-select effect adopts the newest
          // existing thread, so the retry would visibly jump threads. Do this
          // before the delete round-trip so the composer never shows the
          // already-failed thread while the request is in flight.
          startNewThread();
          await api.deleteConversation(createdId).catch(() => undefined);
          void queryClient.invalidateQueries({
            queryKey: CONVERSATIONS_QUERY_KEY,
          });
        }
        throw err;
      }
    },
    onSuccess: ({ runId, conversationId, created, purged }, variables) => {
      recordUxEvent("message_sent", {
        surface,
        mode,
        temporary: variables.temporaryMode,
        tools: variables.allowTools,
        created,
      });
      retryNeedsNewConversation.current = false;
      if (created) setConversationId(conversationId);
      if (purged) setTemporaryNewId(conversationId);
      startRun(runId, conversationId);
      void queryClient.invalidateQueries({
        queryKey: ["messages", conversationId],
      });
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
    },
    onError: () => {
      recordUxEvent("error_shown", { surface, phase: "send" });
    },
  });

  const cancel = useMutation({
    mutationFn: (id: string) => api.cancelRun(id),
  });

  // Edit a user turn and regenerate from it. The backend removes that turn and
  // everything after it, then starts a fresh Run, so this mirrors `sendMessage`
  // exactly: adopt the new run and refresh the transcript.
  const editMessage = useMutation({
    mutationFn: async ({
      messageId,
      content,
    }: {
      messageId: string;
      content: string;
    }) => {
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
      void queryClient.invalidateQueries({
        queryKey: ["messages", activeConversationId],
      });
      void queryClient.invalidateQueries({
        queryKey: ["runs", activeConversationId],
      });
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
    },
  });

  const forkConversation = useMutation({
    mutationFn: async (messageId: string) => {
      if (!activeConversationId) throw new Error("no conversation");
      return api.forkConversation(activeConversationId, messageId);
    },
    onSuccess: (id) => {
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
      // Switch to the fork. The thread list is invalidated first so the
      // sidebar has the new row by the time the transcript loads.
      setConversationId(id);
    },
  });

  // Terminal status: refresh persisted history, then retire the live view
  // once the assistant message lands (the Transcript hides it either way).
  const status = liveStatus;
  useEffect(() => {
    if (!liveRunId || !status || !isTerminal(status)) return;
    if (!recordedCompletions.current.has(liveRunId)) {
      recordedCompletions.current.add(liveRunId);
      recordUxEvent("run_completed", { surface, status });
    }
    const target = liveConversationId ?? activeConversationId;
    void queryClient.invalidateQueries({ queryKey: ["messages", target] });
    void queryClient.invalidateQueries({ queryKey: ["runs", target] });
    void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
    void queryClient.invalidateQueries({ queryKey: ["diagnostics"] });
    const timer = window.setTimeout(() => clearLive(liveRunId), 1500);
    return () => window.clearTimeout(timer);
  }, [
    status,
    liveRunId,
    liveConversationId,
    activeConversationId,
    queryClient,
    clearLive,
    surface,
  ]);

  // Approval visibility is a journey in the UX baseline: how often does a run
  // block, and in which surface? Count once per approval id.
  useEffect(() => {
    const id = liveApproval?.approvalId;
    if (!id || recordedApprovals.current.has(id)) return;
    recordedApprovals.current.add(id);
    recordUxEvent("approval_shown", {
      surface,
      tool: liveApproval.name,
    });
  }, [liveApproval, surface]);

  // A temporary run in a brand-new thread deletes that thread. Wait until the
  // refreshed list no longer carries it before letting go, so the clear cannot
  // race the refetch and leave the composer pointing at a deleted id (or, worse,
  // adopting the wrong thread from the stale list).
  useEffect(() => {
    if (!temporaryNewId || purgedConversation.isPending) return;
    // `get_conversation` returns null once the row is gone. The invalidation
    // after a terminal Run refreshes this query, so the clear cannot race the
    // refetch and leave the composer pointing at a deleted id.
    if (purgedConversation.data === null) {
      setTemporaryNewId(null);
      setConversationId(
        conversationId === temporaryNewId ? null : conversationId,
      );
    }
  }, [
    temporaryNewId,
    conversationId,
    purgedConversation.isPending,
    purgedConversation.data,
    setConversationId,
  ]);

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
    // Merge by id: a refetch of the newest page shifts its window forward, and
    // a plain concatenation would drop the row that fell off the bottom.
    const byId = new Map<string, MessageView>();
    for (const message of olderMessages) byId.set(message.id, message);
    for (const message of messages.data ?? []) byId.set(message.id, message);
    const list = [...byId.values()].sort((a, b) => a.seq - b.seq);
    return list.filter((m) => {
      if (m.role === "user") return true;
      if (m.role !== "assistant") return false;
      const text = (m.content ?? "").trim();
      if (text.length > 0) return true;
      return !!m.reasoning?.trim();
    });
  }, [messages.data, olderMessages]);

  const loadOlder = useCallback(async () => {
    if (!activeConversationId || loadingOlder) return;
    // Use the raw page cursors, not `visibleMessages`: filtering out empty
    // assistant turns must not make the next page start after a row that was
    // never fetched.
    const oldest = olderMessages[0]?.seq ?? messages.data?.[0]?.seq;
    if (oldest === undefined) return;
    setLoadingOlder(true);
    try {
      const page = await api.listMessages(
        activeConversationId,
        MESSAGE_PAGE_SIZE,
        oldest,
      );
      setOlderMessages((previous) => [...page, ...previous]);
      setHasOlder(page.length >= MESSAGE_PAGE_SIZE);
    } finally {
      setLoadingOlder(false);
    }
  }, [activeConversationId, loadingOlder, messages.data, olderMessages]);

  const busy = liveStatus != null && isBusy(liveStatus);

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
    ) => sendMessage.mutateAsync({ content, attachments, ...options }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [sendMessage.mutateAsync],
  );
  const onEditMessage = useCallback(
    (messageId: string, content: string) =>
      editMessage.mutateAsync({ messageId, content }).then(() => {
        // Editing truncates the transcript; paged-in older rows may include
        // turns the backend just removed.
        setOlderMessages([]);
        setHasOlder(false);
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [editMessage.mutateAsync],
  );
  const onStop = useCallback(() => {
    if (liveRunId) cancel.mutate(liveRunId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [liveRunId, cancel.mutate]);
  // Send text to the running agent. `steer_run` returns false when the Run has
  // already finished, in which case the text becomes an ordinary new turn
  // rather than disappearing into a queue nobody will drain.
  const onSteer = useCallback(
    async (content: string) => {
      if (liveRunId) {
        const accepted = await api.steerRun(liveRunId, content);
        if (accepted) {
          if (activeConversationId) {
            // The engine folds it in at the next turn boundary and persists it
            // then; refresh once so the transcript shows the user turn.
            window.setTimeout(() => {
              void queryClient.invalidateQueries({
                queryKey: ["messages", activeConversationId],
              });
            }, 1500);
          }
          return;
        }
      }
      await onSend(content, [], { temporaryMode: false, allowTools: true });
    },
    [liveRunId, activeConversationId, onSend, queryClient],
  );
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
    if (welcomePicked.current || stats.data === undefined) return;
    welcomePicked.current = true;
    const stamp = stats.data.last_activity_at
      ? Date.parse(stats.data.last_activity_at)
      : Number.NaN;
    setWelcome(
      nextVisit({
        conversationCount: stats.data.count,
        messageCount: stats.data.message_count,
        projectCount: projects.data?.length ?? 0,
        lastActivityAt: Number.isFinite(stamp) ? stamp : null,
        surface,
        projectActive: !!activeProjectId,
      }).welcome,
    );
  }, [activeProjectId, stats.data, projects.data, surface]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {liveApproval && <ApprovalBanner approval={liveApproval} />}
      <PaneErrorBoundary name="Transcript" resetKeys={[activeConversationId]}>
        <Transcript
          // Scroll state (`stuck`, previous message count) belongs to one
          // thread. Reusing the component across a thread switch preserved a
          // scrolled-up offset, so the newly opened thread appeared halfway
          // down its history with auto-follow switched off.
          key={activeConversationId ?? "new"}
          messages={visibleMessages}
          runs={runById}
          models={models.data ?? EMPTY_MODELS}
          liveRunId={liveRunId}
          liveConversationId={liveConversationId}
          loading={activeConversationId !== null && messages.isPending}
          error={messages.isError ? messages.error : null}
          welcome={welcome}
          onEditMessage={onEditMessage}
          onRetry={onRetry}
          onFork={(messageId) =>
            forkConversation.mutateAsync(messageId).then(() => undefined)
          }
          onReload={() => void messages.refetch()}
          editDisabled={busy}
          retryDisabled={busy || editMessage.isPending}
          hasOlder={hasOlder}
          loadingOlder={loadingOlder}
          onLoadOlder={() => void loadOlder()}
        />
      </PaneErrorBoundary>
      <RunStatusBar conversationId={liveConversationId} />
      {(sendMessage.isError ||
        editMessage.isError ||
        projectError ||
        cancel.isError) && (
        <div className="mx-auto w-full max-w-content shrink-0 px-4 pb-1 sm:px-6">
          <ErrorBox
            error={
              sendMessage.error ??
              editMessage.error ??
              cancel.error ??
              projectError
            }
            action={
              sendMessage.isError && sendMessage.variables
                ? t("再送信")
                : editMessage.isError && editMessage.variables
                  ? t("再実行")
                  : undefined
            }
            onAction={
              sendMessage.isError && sendMessage.variables
                ? () => {
                    // Replay the exact payload that failed. Rescanning the
                    // transcript for "the last user turn" re-ran an older turn
                    // (deleting its answer) when the failed send never reached
                    // the database, and did nothing at all on a brand-new thread.
                    if (busy || sendMessage.isPending) return;
                    sendMessage.mutate({
                      ...sendMessage.variables,
                      retryNewConversation: retryNeedsNewConversation.current,
                    });
                  }
                : editMessage.isError && editMessage.variables
                  ? () => {
                      if (busy || editMessage.isPending) return;
                      editMessage.mutate(editMessage.variables);
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
            onSteer={onSteer}
            draftKey={activeConversationId ?? "none"}
          />
        </PaneErrorBoundary>
      </div>
    </div>
  );
}
