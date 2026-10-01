import type { ReactNode } from "react";
import { errorMessage } from "@/lib/errors";
import { cn } from "@/lib/cn";
import { Alert, Skeleton } from "./ui/alert";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";

/** Inline error surface used by panes that keep their own layout. */
export function ErrorBox({
  error,
  variant = "default",
  action,
  onAction,
  className,
}: {
  error: unknown;
  variant?: "default" | "warning";
  action?: string;
  onAction?: () => void;
  className?: string;
}) {
  const message = errorMessage(error);
  if (!message) return null;
  return (
    <Alert
      tone={variant === "warning" ? "warning" : "danger"}
      className={className}
      action={
        action && onAction ? (
          <Button variant="default" size="sm" onClick={onAction} className="shrink-0">
            {action}
          </Button>
        ) : undefined
      }
    >
      {message}
    </Alert>
  );
}

/**
 * Block error surface for a failed read.
 *
 * This used to be a second component with a byte-identical body to
 * [`ErrorBox`]; the only difference was that it took no action slot, which is
 * what `ErrorBox` already allowed. Kept as a name because "this read failed" and
 * "this mutation failed" read differently at the call site, but it is the same
 * visual now.
 */
export function ErrorAlert({ error, className }: { error: unknown; className?: string }) {
  return <ErrorBox error={error} className={className} />;
}

export function EmptyState({
  icon,
  title,
  sub,
  action,
  className,
}: {
  icon?: ReactNode;
  title: string;
  sub?: string;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-line px-6 py-10 text-center",
        className,
      )}
    >
      {icon && <div className="text-faint">{icon}</div>}
      <strong className="text-sm font-medium text-fg">{title}</strong>
      {sub && <span className="max-w-sm text-xs text-muted">{sub}</span>}
      {action && <div className="mt-1">{action}</div>}
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

/**
 * Skeleton placeholder for a data table.
 *
 * Most list panes rendered `EmptyState` whenever `data?.length === 0`, which is
 * indistinguishable from "still loading" — twelve panes flashed "no items yet"
 * and then repainted a frame later. A table-shaped skeleton is the honest
 * placeholder: it says the rows are coming, not that there are none.
 */
export function TableSkeleton({ rows = 4, className }: { rows?: number; className?: string }) {
  return (
    <div
      aria-hidden
      className={cn("overflow-hidden rounded-lg border border-line", className)}
    >
      <div className="border-b border-line bg-inset px-3 py-1.5">
        <Skeleton className="h-3 w-24" />
      </div>
      {Array.from({ length: rows }, (_, i) => (
        <div
          key={i}
          className={cn(
            "flex items-center gap-3 px-3 py-2",
            i > 0 && "border-t border-line",
          )}
        >
          <Skeleton className="h-3 w-1/4" />
          <Skeleton className="h-3 w-1/3" />
          <Skeleton className="h-3 w-1/5" />
        </div>
      ))}
    </div>
  );
}

/** Skeleton placeholder for a card-shaped list (agents, providers, runtimes). */
export function ListSkeleton({ rows = 3, className }: { rows?: number; className?: string }) {
  return (
    <div aria-hidden className={cn("flex flex-col gap-3", className)}>
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="rounded-lg border border-line p-3">
          <Skeleton className="h-3.5 w-1/3" />
          <Skeleton className="mt-2 h-3 w-2/3" />
        </div>
      ))}
    </div>
  );
}
