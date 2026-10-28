/// One shared minute clock for relative timestamps.
///
/// `relativeTime()` is a pure formatter: without a ticking "now" a sidebar row
/// that said "たった今" kept saying it hours later. Every timestamp on screen
/// subscribes to the same timer, so a transcript with 200 messages still runs
/// one interval and re-renders once per minute, not once per row.

import { useSyncExternalStore } from "react";

const TICK_MS = 60_000;

let currentNow = Date.now();
const listeners = new Set<() => void>();
let timer: number | null = null;

function startTimer(): void {
  if (timer !== null || typeof window === "undefined") return;
  timer = window.setInterval(() => {
    currentNow = Date.now();
    for (const listener of listeners) listener();
  }, TICK_MS);
}

function stopTimer(): void {
  if (timer === null) return;
  window.clearInterval(timer);
  timer = null;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  startTimer();
  // A component can mount long after the last tick; make sure it starts from
  // the real clock instead of the timestamp from when the module loaded.
  currentNow = Date.now();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) stopTimer();
  };
}

function getSnapshot(): number {
  return currentNow;
}

/** Milliseconds since the epoch, refreshed once per minute. */
export function useNow(): number {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
