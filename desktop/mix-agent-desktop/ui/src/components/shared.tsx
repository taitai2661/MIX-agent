import type { ReactNode } from "react";
import { Alert } from "./ui/alert";

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
  const message = error instanceof Error ? error.message : String(error ?? "");
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
