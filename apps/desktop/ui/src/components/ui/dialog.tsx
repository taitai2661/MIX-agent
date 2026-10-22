import { forwardRef, useCallback, useRef, useState } from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { t } from "@/lib/i18n";
import { cn } from "@/lib/cn";
import { Button } from "./button";

export const Dialog = DialogPrimitive.Root;

/**
 * Modal surface.
 *
 * The app had no dialog at all: four destructive actions called
 * `window.confirm`, which in a Tauri WebView opens a system alert that ignores
 * the theme, sits outside the window, and cannot be styled. The command palette
 * had hand-rolled its own overlay with no focus trap and a swallowed Tab. Both
 * are now this.
 */
export const DialogContent = forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content>
>(function DialogContent({ className, children, ...props }, ref) {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-scrim backdrop-blur-[2px] animate-fade" />
      <DialogPrimitive.Content
        ref={ref}
        className={cn(
          "fixed top-1/2 left-1/2 z-50 max-h-[85vh] w-full max-w-md -translate-x-1/2 -translate-y-1/2",
          "overflow-y-auto rounded-2xl border border-line bg-elevated p-5 shadow-pop animate-rise",
          className,
        )}
        {...props}
      >
        {children}
        <DialogPrimitive.Close
          className="absolute top-2.5 right-2.5 text-faint transition-colors hover:text-fg"
          title={t("閉じる")}
          aria-label={t("閉じる")}
        >
          <X size={14} />
        </DialogPrimitive.Close>
      </DialogPrimitive.Content>
    </DialogPrimitive.Portal>
  );
});

export function DialogHeader({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      className={cn("mb-3 flex flex-col gap-1 pe-6", className)}
      {...props}
    />
  );
}

export function DialogTitle({
  className,
  ...props
}: React.ComponentProps<"h2">) {
  return (
    <DialogPrimitive.Title
      className={cn(
        "text-base font-semibold tracking-tight text-fg",
        className,
      )}
      {...props}
    />
  );
}

export function DialogDescription({
  className,
  ...props
}: React.ComponentProps<"p">) {
  return (
    <DialogPrimitive.Description
      className={cn("text-sm text-muted", className)}
      {...props}
    />
  );
}

export function DialogFooter({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      className={cn("mt-4 flex items-center justify-end gap-2", className)}
      {...props}
    />
  );
}

export interface ConfirmRequest {
  title: string;
  description?: string;
  confirmLabel: string;
  cancelLabel?: string;
  /** `danger` also makes the dialog undismissable by backdrop or Escape: for a
   *  destructive action, an absent answer must never be a silent "yes". */
  tone?: "default" | "danger";
}

/**
 * Promise-based confirmation, replacing `if (window.confirm(msg)) mutate()`.
 *
 * Both answers settle the promise, so a caller reads as one expression and
 * cannot forget the cancel path. Render [`ConfirmDialog`] once per view that
 * uses it; `confirm` is stable, so it is safe in a `useCallback` dependency.
 */
export function useConfirm() {
  const [request, setRequest] = useState<ConfirmRequest | null>(null);
  // Held in a ref rather than read out of `request` state: the resolver is
  // called from the click handler, which may run before React has re-rendered
  // with the new request.
  const resolver = useRef<((ok: boolean) => void) | null>(null);

  const settle = useCallback((ok: boolean) => {
    setRequest(null);
    resolver.current?.(ok);
    resolver.current = null;
  }, []);

  const confirm = useCallback((next: ConfirmRequest) => {
    // A second request while one is pending resolves the first as declined, so
    // its `await` can never hang.
    resolver.current?.(false);
    setRequest(next);
    return new Promise<boolean>((resolve) => {
      resolver.current = resolve;
    });
  }, []);

  const dialog = (
    <Dialog
      open={request !== null}
      onOpenChange={(open) => !open && settle(false)}
    >
      {request && (
        <DialogContent
          onInteractOutside={(e: Event) => {
            if (request.tone === "danger") e.preventDefault();
          }}
          onEscapeKeyDown={(e: KeyboardEvent) => {
            if (request.tone === "danger") e.preventDefault();
          }}
          className="max-w-sm"
        >
          <DialogHeader>
            <DialogTitle>{request.title}</DialogTitle>
            {request.description && (
              <DialogDescription>{request.description}</DialogDescription>
            )}
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" size="sm" onClick={() => settle(false)}>
              {request.cancelLabel ?? t("キャンセル")}
            </Button>
            <Button
              variant={request.tone === "danger" ? "danger" : "primary"}
              size="sm"
              onClick={() => settle(true)}
            >
              {request.confirmLabel}
            </Button>
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
  );

  return { confirm, dialog };
}
