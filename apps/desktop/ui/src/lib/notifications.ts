/// Desktop notifications that can carry the user back to the right place.
///
/// The Tauri notification plugin only exposes `onAction` on mobile, so a
/// desktop click cannot be observed directly. The workaround is deliberately
/// small: sending a notification records its destination; the window's next
/// `focus` event consumes it. Clicking a notification focuses the app, which
/// makes the jump work on macOS/Windows/Linux without a platform-specific
/// native hook. The record expires quickly so an unrelated focus minutes later
/// never teleports the user.

import {
  isPermissionGranted,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { getJSON, removeKey, setJSON } from "./storage";

export type NotificationTarget =
  | { kind: "thread"; conversationId: string }
  | { kind: "approval"; conversationId: string };

interface StoredTarget {
  target: NotificationTarget;
  at: number;
}

const KEY = "mix-agent-notification-target";
/** How long after the notification a focus may still count as its click. */
const MAX_AGE_MS = 2 * 60 * 1000;

function isTarget(value: unknown): value is NotificationTarget {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  if (
    record.kind !== "thread" && record.kind !== "approval"
  ) {
    return false;
  }
  return typeof record.conversationId === "string" && !!record.conversationId;
}

function isStored(value: unknown): value is StoredTarget {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return isTarget(record.target) && typeof record.at === "number";
}

/**
 * Show a notification and remember where a click should land.
 *
 * Returns false when permission is missing or the platform rejects the send;
 * callers use that to avoid claiming a click target that cannot exist.
 */
export async function notifyWithTarget(input: {
  title: string;
  body: string;
  target: NotificationTarget;
}): Promise<boolean> {
  try {
    if (!(await isPermissionGranted())) return false;
    setJSON(KEY, { target: input.target, at: Date.now() } satisfies StoredTarget);
    sendNotification({ title: input.title, body: input.body });
    return true;
  } catch {
    removeKey(KEY);
    return false;
  }
}

/** Consume a fresh notification destination, if any. */
export function consumeNotificationTarget(
  now = Date.now(),
): NotificationTarget | null {
  const stored = getJSON<StoredTarget | null>(KEY, null, isStored);
  if (!stored || now - stored.at > MAX_AGE_MS) {
    removeKey(KEY);
    return null;
  }
  removeKey(KEY);
  return stored.target;
}

/** Test helper: clear without consuming. */
export function clearNotificationTarget(): void {
  removeKey(KEY);
}
