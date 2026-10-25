/// Live run-event store. A single global Tauri listener feeds events here;
/// deltas are buffered and flushed on a 60ms timer so token streams cause at
/// most ~16 renders per second regardless of event volume.

import { create } from "zustand";
import { applyRunEvent, createTimeline, type RunTimeline } from "./runEvents";
import type { RunEventPayload } from "./api";

const FLUSH_MS = 60;

interface RunStoreState {
  liveRunId: string | null;
  /** Conversation the live run belongs to, so the view can filter it. */
  conversationId: string | null;
  timeline: RunTimeline | null;
  /** Begin tracking a run (called right after `send_message` resolves). */
  startRun: (runId: string, conversationId: string | null) => void;
  /** Feed one event; ignored unless it belongs to the live run. */
  push: (event: RunEventPayload) => void;
  /** Drop the live view (e.g. after the persisted message is confirmed). */
  clear: () => void;
}

let buffer: RunEventPayload[] = [];
let timer: number | null = null;
/** Events that arrived before `startRun` (or for a not-yet-live run). */
const pendingByRun = new Map<string, RunEventPayload[]>();

function scheduleFlush() {
  if (timer !== null) return;
  timer = window.setTimeout(() => {
    timer = null;
    const events = buffer;
    buffer = [];
    if (events.length === 0) return;
    const state = useRunEvents.getState();
    if (!state.timeline || !state.liveRunId) return;
    let timeline = state.timeline;
    for (const event of events) {
      timeline = applyRunEvent(timeline, event);
    }
    useRunEvents.setState({ timeline });
  }, FLUSH_MS);
}

export const useRunEvents = create<RunStoreState>((set, get) => ({
  liveRunId: null,
  conversationId: null,
  timeline: null,
  startRun: (runId, conversationId) => {
    buffer = [];
    if (timer !== null) {
      window.clearTimeout(timer);
      timer = null;
    }
    // Drain events that arrived before the run was registered, and drop every
    // other run's queue: a new run supersedes whatever came before it.
    const queued = pendingByRun.get(runId) ?? [];
    pendingByRun.clear();
    let timeline = createTimeline(runId);
    for (const event of queued) {
      timeline = applyRunEvent(timeline, event);
    }
    set({ liveRunId: runId, conversationId, timeline });
  },
  push: (event) => {
    const { liveRunId } = get();
    if (!liveRunId || event.run_id !== liveRunId) {
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
    if (timer !== null) {
      window.clearTimeout(timer);
      timer = null;
    }
    set({ liveRunId: null, conversationId: null, timeline: null });
  },
}));
