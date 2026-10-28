import { useEffect, useRef, useSyncExternalStore } from "react";
import { Clock3 } from "lucide-react";
import { t, tf } from "@/lib/i18n";
import { cn } from "@/lib/cn";

/**
 * One clock for every countdown on screen.
 *
 * Each card used to start its own `setInterval`, so an approval queue with ten
 * items ran ten timers — and none of them stopped when the deadline passed.
 * `useSyncExternalStore` lets all subscribers share a single one-second tick;
 * the timer is created on the first subscriber and cleared with the last.
 */
const TICK_MS = 1000;
let currentNow = Date.now();
const listeners = new Set<() => void>();
/** Subscriber id → deadline. The timer stops once no deadline is in the future. */
const deadlines = new Map<symbol, number>();
let timer: number | null = null;

function hasFutureDeadline(): boolean {
  const now = Date.now();
  for (const deadline of deadlines.values()) {
    if (deadline > now) return true;
  }
  return false;
}

function stopTimer() {
  if (timer === null) return;
  window.clearInterval(timer);
  timer = null;
}

function ensureTimer() {
  if (timer !== null || !hasFutureDeadline()) return;
  currentNow = Date.now();
  timer = window.setInterval(tick, TICK_MS);
}

/** Recompute the shared snapshot after a new deadline is registered. Without
 *  this the first paint used the clock from the last tick (or page load) and
 *  showed a countdown that was too long until the next second. */
function refreshNow() {
  currentNow = Date.now();
  for (const listener of listeners) listener();
}

function tick() {
  currentNow = Date.now();
  for (const listener of listeners) listener();
  // No approval on screen can change any more: do not keep the process awake.
  if (!hasFutureDeadline()) stopTimer();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  ensureTimer();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) stopTimer();
  };
}

function getSnapshot() {
  return currentNow;
}

/**
 * Milliseconds left before an approval prompt is auto-denied.
 *
 * Returns `null` when the backend did not send a deadline (older build) so the
 * UI can fall back to its previous, timeless copy instead of inventing one.
 * Returns 0 once expired — the server treats expiry as a denial, so the card
 * must stop offering "allow" at that point.
 */
export function useApprovalRemaining(expiresAt?: string | null): number | null {
  const id = useRef(Symbol("approval-deadline"));
  const now = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  useEffect(() => {
    const subscriber = id.current;
    const deadline = expiresAt ? Date.parse(expiresAt) : Number.NaN;
    if (!Number.isFinite(deadline)) {
      deadlines.delete(subscriber);
      return;
    }
    deadlines.set(subscriber, deadline);
    ensureTimer();
    refreshNow();
    return () => {
      deadlines.delete(subscriber);
      if (listeners.size === 0) stopTimer();
    };
  }, [expiresAt]);

  if (!expiresAt) return null;
  const deadline = Date.parse(expiresAt);
  if (!Number.isFinite(deadline)) return null;
  return Math.max(0, deadline - now);
}

export function ApprovalCountdown({
  expiresAt,
  className,
}: {
  expiresAt?: string | null;
  className?: string;
}) {
  const remaining = useApprovalRemaining(expiresAt);
  if (remaining === null) return null;

  const expired = remaining <= 0;
  const seconds = Math.ceil(remaining / 1000);
  const label = expired
    ? t("期限切れ（拒否されます）")
    : tf("残り {time}", {
        time: `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`,
      });
  const urgent = !expired && seconds <= 60;

  return (
    <span
      role="timer"
      aria-live="off"
      className={cn(
        "inline-flex shrink-0 items-center gap-1 font-mono text-2xs tabular-nums",
        expired ? "text-danger" : urgent ? "text-warning" : "text-faint",
        className,
      )}
      title={
        expired
          ? t("承認が期限切れになったため、この操作は拒否されました。")
          : t("この時刻を過ぎると自動的に拒否されます。")
      }
    >
      <Clock3 size={11} aria-hidden="true" />
      {label}
    </span>
  );
}
