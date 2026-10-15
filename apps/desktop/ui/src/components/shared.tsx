import { useState, type ReactNode } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { errorMessage } from "@/lib/errors";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { Alert, Skeleton } from "./ui/alert";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { CopyButton } from "./CopyButton";

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
  const [expanded, setExpanded] = useState(false);
  const message = errorMessage(error);
  if (!message) return null;

  const lines = message.split("\n");
  const isMultiLine = lines.length > 1;
  const isLong = message.length > 160;
  const showExpandable = isMultiLine || isLong;

  return (
    <Alert
      tone={variant === "warning" ? "warning" : "danger"}
      className={cn("items-start", className)}
      action={
        <div className="flex shrink-0 items-center gap-1">
          {action && onAction && (
            <Button
              variant="default"
              size="sm"
              onClick={onAction}
              className="shrink-0"
            >
              {action}
            </Button>
          )}
          <CopyButton value={message} label={t("エラー内容をコピー")} />
        </div>
      }
    >
      <div className="flex min-w-0 flex-1 flex-col gap-1 text-xs">
        {showExpandable ? (
          <div>
            <div className="font-medium text-fg break-words">
              {lines[0] || message}
            </div>
            <button
              type="button"
              onClick={() => setExpanded((prev) => !prev)}
              className="mt-1 inline-flex items-center gap-1 font-mono text-2xs text-faint transition hover:text-muted"
            >
              {expanded ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
              <span>{expanded ? t("詳細を折りたたむ") : t("詳細を表示")}</span>
            </button>
            {expanded && (
              <pre className="mt-1.5 max-h-48 overflow-auto rounded bg-inset/80 p-2 font-mono text-2xs text-muted whitespace-pre-wrap break-all border border-line/40">
                {message}
              </pre>
            )}
          </div>
        ) : (
          <div className="break-words font-medium">{message}</div>
        )}
      </div>
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
  compact = false,
  className,
}: {
  icon?: ReactNode;
  title: string;
  sub?: string;
  action?: ReactNode;
  /** Dense variant for side panels, popovers and sidebar lists, where the
   *  full boxed state would dwarf the content it replaces. */
  compact?: boolean;
  className?: string;
}) {
  if (compact) {
    return (
      <div className={cn("px-2 py-6 text-center", className)}>
        <strong className="text-xs font-medium text-muted">{title}</strong>
        {sub && <p className="m-0 mt-1 text-2xs text-faint">{sub}</p>}
        {action && <div className="mt-2">{action}</div>}
      </div>
    );
  }
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-1.5 rounded-xl border border-dashed border-line-strong/60 bg-surface/50 px-6 py-10 text-center",
        className,
      )}
    >
      {icon && <div className="mb-1 grid size-9 place-items-center rounded-xl bg-inset text-faint">{icon}</div>}
      <strong className="text-sm font-semibold text-fg">{title}</strong>
      {sub && <span className="max-w-sm text-xs leading-5 text-muted">{sub}</span>}
      {action && <div className="mt-2">{action}</div>}
    </div>
  );
}

/**
 * One recipe for "raw payload shown in a card": tool arguments, JSON dumps,
 * skill bodies. Five variants of this block were in circulation (bordered and
 * not, `leading-4` and `leading-5`, `text-muted` and `text-fg`).
 */
export const payloadBlock =
  "m-0 overflow-auto whitespace-pre-wrap rounded-md border border-line bg-inset px-2 py-1.5 font-mono text-xs leading-5 text-muted";

/** Compact inline error, for a form row that already owns its layout.
 *
 * A Tauri `invoke` rejects with the bare `String` the command returned, so
 * `String(err)` renders `[object Object]` for anything that is not already a
 * string. Every caller here used to do exactly that, which is how a
 * failed provider creation once showed up as an empty badge. */
export function ErrorBadge({ error }: { error: unknown }) {
  const message = errorMessage(error);
  if (!message) return null;
  return (
    <Badge tone="danger" title={message} className="cursor-help">
      {message}
    </Badge>
  );
}

/**
 * Skeleton placeholder for a data table.
 *
 * Most list panes rendered `EmptyState` whenever `data?.length === 0`, which is
 * indistinguishable from "still loading" — twelve panes flashed "no items yet"
 * and then repainted a frame later. A table-shaped skeleton is the honest
 * placeholder: it says the rows are coming, not that there are none.
 */
export function TableSkeleton({ rows = 4, className, label }: { rows?: number; className?: string; label?: string }) {
  return (
    <div
      role="status"
      aria-busy="true"
      aria-label={label}
      className={cn("overflow-hidden rounded-xl border border-line", className)}
    >
      {label && <span className="sr-only">{label}</span>}
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
export function ListSkeleton({ rows = 3, className, label }: { rows?: number; className?: string; label?: string }) {
  return (
    <div role="status" aria-busy="true" aria-label={label} className={cn("flex flex-col gap-3", className)}>
      {label && <span className="sr-only">{label}</span>}
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="rounded-xl border border-line p-3">
          <Skeleton className="h-3.5 w-1/3" />
          <Skeleton className="mt-2 h-3 w-2/3" />
        </div>
      ))}
    </div>
  );
}
