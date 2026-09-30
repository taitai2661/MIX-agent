import type { ReactNode } from "react";
import { errorMessage } from "@/lib/errors";
import { Alert } from "./ui/alert";
import { Badge } from "./ui/badge";

/** Inline error surface used by panes that keep their own layout. */
export function ErrorBox({
  error,
  variant = "default",
  action,
  onAction,
}: {
  error: unknown;
  variant?: "default" | "warning";
  action?: string;
  onAction?: () => void;
}) {
  const message = errorMessage(error);
  if (!message) return null;
  return (
    <Alert
      tone={variant === "warning" ? "warning" : "danger"}
      action={
        action && onAction ? (
          <button
            type="button"
            onClick={onAction}
            className="h-7 shrink-0 rounded-md border border-line bg-surface px-2.5 text-xs font-medium text-fg transition-colors hover:bg-hover"
          >
            {action}
          </button>
        ) : undefined
      }
    >
      {message}
    </Alert>
  );
}

export function EmptyState({ icon, title, sub }: { icon?: ReactNode; title: string; sub?: string }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-line px-6 py-10 text-center">
      {icon && <div className="text-faint">{icon}</div>}
      <strong className="text-sm font-medium text-fg">{title}</strong>
      {sub && <span className="max-w-sm text-xs text-muted">{sub}</span>}
    </div>
  );
}

/** Compact inline error, for a form row that already owns its layout.
 *
 * A Tauri `invoke` rejects with the bare `String` the command returned, so
 * `String(err)` renders `[object Object]` for anything that is not already a
 * string. Every caller here used to do exactly that, which is how a
 * failed provider creation once showed up as an empty badge. */
export function ErrorBadge({ error }: { error: unknown }) {
  const message = errorMessage(error);
  if (!message) return null;
  return <Badge tone="danger">{message}</Badge>;
}

/** Block error surface for a failed read, mirroring [`ErrorBox`]. */
export function ErrorAlert({ error }: { error: unknown }) {
  const message = errorMessage(error);
  if (!message) return null;
  return <Alert tone="danger">{message}</Alert>;
}
