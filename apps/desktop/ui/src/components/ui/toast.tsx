import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { useToasts, type ToastItem } from "@/lib/toast";
import { t } from "@/lib/i18n";
import { Button } from "./button";

function ToastRow({ toast }: { toast: ToastItem }) {
  const dismiss = useToasts((s) => s.dismiss);
  // Undo windows are short; pausing while the pointer or keyboard focus is on
  // the toast gives a reader enough time to act (WCAG 2.2.1). The timer
  // restarts from full on unpause rather than tracking remaining time: the
  // only cost is a slightly longer toast.
  const [paused, setPaused] = useState(false);

  useEffect(() => {
    if (paused) return;
    const timer = setTimeout(() => dismiss(toast.id), toast.timeoutMs);
    return () => clearTimeout(timer);
  }, [dismiss, paused, toast.id, toast.seq, toast.timeoutMs]);

  return (
    <div
      className="pointer-events-auto flex w-full max-w-sm items-center gap-2 rounded-lg border border-line bg-surface px-3 py-2 text-xs text-fg shadow-pop"
      onPointerEnter={() => setPaused(true)}
      onPointerLeave={() => setPaused(false)}
      onFocusCapture={() => setPaused(true)}
      onBlurCapture={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
          setPaused(false);
        }
      }}
    >
      {/* The live region contains only text. A button inside `role="status"`
          made screen readers announce the control as part of the message. */}
      <span
        role="status"
        aria-live="polite"
        aria-atomic="true"
        className="min-w-0 flex-1 break-words"
      >
        {toast.message}
      </span>
      {toast.action && (
        <Button
          variant="ghost"
          size="xs"
          className="shrink-0 font-medium text-brand"
          onClick={() => {
            try {
              toast.action?.onClick();
            } finally {
              // Undo must suppress `onDismiss` (which would perform the
              // deferred delete/save the action just cancelled).
              dismiss(toast.id, { runOnDismiss: false });
            }
          }}
        >
          {toast.action.label}
        </Button>
      )}
      <Button
        variant="ghost"
        size="icon-sm"
        className="shrink-0 text-faint"
        onClick={() => dismiss(toast.id)}
        aria-label={t("閉じる")}
        title={t("閉じる")}
      >
        <X size={12} aria-hidden="true" />
      </Button>
    </div>
  );
}

/** Mount once at the app root; renders the queue bottom-centre above content. */
export function Toaster() {
  const toasts = useToasts((s) => s.toasts);
  if (toasts.length === 0) return null;
  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-4 z-50 flex flex-col items-center gap-2 px-4">
      {toasts.map((toast) => (
        <ToastRow key={`${toast.id}:${toast.seq}`} toast={toast} />
      ))}
    </div>
  );
}
