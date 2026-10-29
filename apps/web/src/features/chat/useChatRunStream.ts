import { ja } from "@/app/strings";
import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";

import {
  parseRunEvent,
  TERMINAL_RUN_STATUSES,
  type RunEvent,
  type RunStatus,
} from "./types";

export type ChatRunStreamState = {
  events: RunEvent[];
  streamText: string;
  reasoning: string;
  streamConnected: boolean;
};

/** The fields the stream needs from a stored conversation run. */
export type StoredRunSummary = {
  id: string;
  status?: string;
};

/**
 * Owns the `/runs/:id/events` EventSource lifecycle for the active run.
 *
 * Text and reasoning fragments are buffered in refs and flushed once per
 * animation frame so the transcript is not re-rendered for every SSE chunk.
 * Run-terminal invalidations live here too, because they are part of the
 * stream contract rather than the page layout.
 */
export function useChatRunStream({
  conversationId,
  runId,
  storedRuns,
  onFinish,
  onStreamLost,
}: {
  conversationId: string | undefined;
  runId: string;
  storedRuns: readonly StoredRunSummary[] | undefined;
  /** Called when a `done` event or a fatal stream error ends the run. */
  onFinish: () => void;
  /** Called after repeated reconnect failures. */
  onStreamLost: (error: Error) => void;
}): ChatRunStreamState {
  const qc = useQueryClient();
  const [streamText, setStreamText] = useState("");
  const [reasoning, setReasoning] = useState("");
  const [events, setEvents] = useState<RunEvent[]>([]);
  const [streamConnected, setStreamConnected] = useState(false);
  const eventSourceRef = useRef<EventSource | null>(null);
  const streamTextBuffer = useRef("");
  const reasoningBuffer = useRef("");
  const streamFrame = useRef<number | null>(null);
  const storedRunsRef = useRef(storedRuns);
  const finishRef = useRef(onFinish);
  const lostRef = useRef(onStreamLost);

  useEffect(() => {
    storedRunsRef.current = storedRuns;
  }, [storedRuns]);
  useEffect(() => {
    finishRef.current = onFinish;
    lostRef.current = onStreamLost;
  }, [onFinish, onStreamLost]);

  const flushStreamBuffers = useCallback(() => {
    if (streamFrame.current !== null) {
      cancelAnimationFrame(streamFrame.current);
      streamFrame.current = null;
    }
    setStreamText(streamTextBuffer.current);
    setReasoning(reasoningBuffer.current);
  }, []);

  const scheduleStreamFlush = useCallback(() => {
    if (streamFrame.current !== null) return;
    streamFrame.current = requestAnimationFrame(() => {
      streamFrame.current = null;
      setStreamText(streamTextBuffer.current);
      setReasoning(reasoningBuffer.current);
    });
  }, []);

  const resetStreamBuffers = useCallback(() => {
    if (streamFrame.current !== null) {
      cancelAnimationFrame(streamFrame.current);
      streamFrame.current = null;
    }
    streamTextBuffer.current = "";
    reasoningBuffer.current = "";
    setStreamText("");
    setReasoning("");
  }, []);

  // Opening another conversation starts from a clean stream.
  useEffect(() => {
    setEvents([]);
    setStreamConnected(false);
    resetStreamBuffers();
  }, [conversationId, resetStreamBuffers]);

  useEffect(() => {
    if (!runId) return;
    // Opening a finished conversation used to attach an EventSource that
    // replayed every stored token, briefly rendering the last answer a second
    // time. Terminal runs have nothing to stream; the messages query is enough.
    const storedRun = storedRunsRef.current?.find((r) => r.id === runId);
    if (
      storedRun &&
      TERMINAL_RUN_STATUSES.includes(storedRun.status as RunStatus)
    ) {
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
    const watchdog = setTimeout(
      () => {
        if (source.readyState === EventSource.OPEN) {
          source.close();
          setStreamConnected(false);
        }
      },
      60 * 60 * 1000,
    );
    const invalidateMessages = () => {
      qc.invalidateQueries({ queryKey: ["messages", conversationId] });
    };
    const invalidateRun = () => {
      qc.invalidateQueries({ queryKey: ["run", runId] });
    };
    const finish = () => {
      clearTimeout(watchdog);
      source.close();
      flushStreamBuffers();
      finishRef.current();
      setStreamConnected(false);
      invalidateMessages();
      invalidateRun();
    };
    const onActivity = (event: Event) => {
      if (!(event instanceof MessageEvent) || typeof event.data !== "string")
        return;
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
        reasoningBuffer.current = reasoningBuffer.current
          ? reasoningBuffer.current + "\n\n"
          : "";
        flushStreamBuffers();
        setEvents((xs) => [...xs, v]);
        // A new step was committed on the server; refresh steps/budget.
        invalidateRun();
      } else {
        setEvents((xs) => [...xs, v]);
        if (v.kind === "approval") {
          invalidateRun();
        }
        if (v.kind === "status" || v.kind === "context_summary")
          invalidateRun();
        if (v.kind === "message") {
          streamTextBuffer.current = "";
          flushStreamBuffers();
          invalidateMessages();
        }
      }
      if (["tool_started", "tool_result", "plan"].includes(v.kind))
        invalidateRun();
      // update_plan mirrors the conversation todo list; refresh the pane.
      if (v.kind === "plan") invalidateMessages();
      if (
        ["tool_started", "tool_result", "approval", "status"].includes(v.kind)
      )
        qc.invalidateQueries({ queryKey: ["tool-calls", conversationId] });
      if (v.kind === "browser_frame")
        qc.invalidateQueries({ queryKey: ["browser-frames", runId] });
      if (
        v.kind === "checkpoint_saved" ||
        v.kind === "budget_extension_requested" ||
        v.kind === "budget_extension_resolved" ||
        v.kind === "stagnation_detected" ||
        v.kind === "verification_required" ||
        v.kind === "phase_advanced"
      ) {
        invalidateRun();
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
        finishRef.current();
        lostRef.current(new Error(ja.streamLost));
        invalidateMessages();
        invalidateRun();
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
  }, [
    conversationId,
    flushStreamBuffers,
    qc,
    resetStreamBuffers,
    runId,
    scheduleStreamFlush,
  ]);

  return { events, streamText, reasoning, streamConnected };
}
