/// Live run-event store. A single global Tauri listener feeds events here;
/// deltas are buffered and flushed on a 60ms timer so token streams cause at
/// most ~16 renders per second regardless of event volume.
///
/// Timelines are kept per conversation, not in one global slot. Switching
/// threads used to zero the only live run: a task still running in the
/// background lost its stream (and its sidebar marker) the moment the user
/// opened another thread or sent a message there.

import { create } from "zustand";
import { applyRunEvent, createTimeline, type RunTimeline } from "./runEvents";
import type { RunEventPayload } from "./api";
import { useUi } from "./ui";

const FLUSH_MS = 60;

export interface LiveRunEntry {
  runId: string;
  timeline: RunTimeline;
}

interface RunStoreState {
  /** Most recently started run, kept for backwards-compatible consumers. */
  liveRunId: string | null;
  /** Conversation of the most recently started run. */
  conversationId: string | null;
  timeline: RunTimeline | null;
  /** Every tracked run, keyed by conversation id (or run id when there is no
   *  conversation yet). */
  byConversation: Record<string, LiveRunEntry>;
  /** Begin tracking a run (called right after `send_message` resolves). */
  startRun: (runId: string, conversationId: string | null) => void;
  /** Feed one event; ignored unless its run is tracked. */
  push: (event: RunEventPayload) => void;
  /** Drop every live view (tests, workspace switch). */
  clear: () => void;
  /** Retire one finished run without touching other threads. */
  clearRun: (runId: string) => void;
}

let buffer: RunEventPayload[] = [];
let timer: number | null = null;
/** Events that arrived before `startRun` (or for a not-yet-live run). */
const pendingByRun = new Map<string, RunEventPayload[]>();
/** run id → key in `byConversation`. */
const keyByRun = new Map<string, string>();

function scheduleFlush() {
  if (timer !== null) return;
  timer = window.setTimeout(() => {
    timer = null;
    const events = buffer;
    buffer = [];
    if (events.length === 0) return;
    const state = useRunEvents.getState();
    if (!state.liveRunId && Object.keys(state.byConversation).length === 0)
      return;
    let next: Record<string, LiveRunEntry> | null = null;
    for (const event of events) {
      const key = keyByRun.get(event.run_id);
      if (key === undefined) continue;
      const current = (next ?? state.byConversation)[key];
      if (!current || current.runId !== event.run_id) continue;
      if (!next) next = { ...state.byConversation };
      next[key] = {
        ...current,
        timeline: applyRunEvent(current.timeline, event),
      };
    }
    if (!next) return;
    const active = state.liveRunId
      ? next[keyByRun.get(state.liveRunId) ?? ""]
      : null;
    useRunEvents.setState({
      byConversation: next,
      timeline: active?.timeline ?? null,
    });
  }, FLUSH_MS);
}

const EMPTY_ENTRIES: Record<string, LiveRunEntry> = {};

/** Conversation a tracked run belongs to (null when the run was never seen). */
export function conversationForRun(runId: string): string | null {
  return keyByRun.get(runId) ?? null;
}

export const useRunEvents = create<RunStoreState>((set) => ({
  liveRunId: null,
  conversationId: null,
  timeline: null,
  byConversation: EMPTY_ENTRIES,
  startRun: (runId, conversationId) => {
    // The UI store tracks the run whose browser pane a manual close should
    // suppress. It used to have no writer at all, so every dismissal was
    // recorded as "manual" and the next browser tool call reopened the pane.
    // Only a run in the thread the user is looking at may claim the marker;
    // a background run in another thread must not make a close in this one
    // look like it happened elsewhere.
    const ui = useUi.getState();
    if (!conversationId || ui.conversationId === conversationId) {
      ui.setRunId(runId);
    }
    if (timer !== null) {
      window.clearTimeout(timer);
      timer = null;
    }
    const key = conversationId ?? runId;
    keyByRun.set(runId, key);
    // Drain events that arrived before the run was registered. Other runs'
    // queues are left alone: they are still live.
    const queued = pendingByRun.get(runId) ?? [];
    pendingByRun.delete(runId);
    let timeline = createTimeline(runId);
    for (const event of queued) {
      timeline = applyRunEvent(timeline, event);
    }
    set((state) => ({
      liveRunId: runId,
      conversationId,
      timeline,
      byConversation: {
        ...state.byConversation,
        [key]: { runId, timeline },
      },
    }));
  },
  push: (event) => {
    if (!keyByRun.has(event.run_id)) {
      // Queue out-of-order / pre-start events per run instead of dropping.
      const queue = pendingByRun.get(event.run_id) ?? [];
      queue.push(event);
      // Bound the queue so a hot non-live run cannot grow without limit.
      if (queue.length > 2000) queue.splice(0, queue.length - 2000);
      pendingByRun.set(event.run_id, queue);
      // Bound the map itself: runs that never start must not accumulate.
      if (pendingByRun.size > 20) {
        const oldest = pendingByRun.keys().next().value;
        if (oldest !== undefined) pendingByRun.delete(oldest);
      }
      return;
    }
    buffer.push(event);
    scheduleFlush();
  },
  clear: () => {
    buffer = [];
    pendingByRun.clear();
    keyByRun.clear();
    if (timer !== null) {
      window.clearTimeout(timer);
      timer = null;
    }
    useUi.getState().setRunId(null);
    set({
      liveRunId: null,
      conversationId: null,
      timeline: null,
      byConversation: EMPTY_ENTRIES,
    });
  },
  clearRun: (runId) => {
    const key = keyByRun.get(runId);
    if (key === undefined) return;
    // Only the most recently started run clears the UI's dismiss marker; an
    // older run finishing must not clear the marker for the active one.
    if (useUi.getState().runId === runId) useUi.getState().setRunId(null);
    // The conversation key is reused by the next run in the same thread. A
    // delayed clear for run A (1.5s after it finished) must not delete run B's
    // entry, which would silently stop B's stream from updating.
    if (useRunEvents.getState().byConversation[key]?.runId !== runId) {
      keyByRun.delete(runId);
      buffer = buffer.filter((event) => event.run_id !== runId);
      return;
    }
    keyByRun.delete(runId);
    buffer = buffer.filter((event) => event.run_id !== runId);
    set((state) => {
      const next = { ...state.byConversation };
      delete next[key];
      const active = state.liveRunId === runId;
      return {
        byConversation: Object.keys(next).length ? next : EMPTY_ENTRIES,
        liveRunId: active ? null : state.liveRunId,
        conversationId: active ? null : state.conversationId,
        timeline: active ? null : state.timeline,
      };
    });
  },
}));
