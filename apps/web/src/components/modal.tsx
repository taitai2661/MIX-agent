import {
  useEffect,
  useRef,
  useState,
  type ComponentProps,
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { Button } from "@/components/button";
import { ja } from "@/app/strings";
import { t } from "@/app/i18n";

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function trapFocus(panel: HTMLElement, event: KeyboardEvent) {
  const focusable = panel.querySelectorAll<HTMLElement>(FOCUSABLE);
  if (!focusable.length) return;
  const first = focusable[0],
    last = focusable[focusable.length - 1];
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first.focus();
  }
}

export function useModalDialog<T extends HTMLElement>(
  panelRef: RefObject<T | null>,
  onCancel: () => void,
) {
  // Keep the latest handler without re-running the focus effect on every
  // parent render (typing in a controlled field used to steal focus).
  const cancelRef = useRef(onCancel);
  cancelRef.current = onCancel;
  useEffect(() => {
    const previouslyFocused =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const preferred =
      panelRef.current?.querySelector<HTMLElement>("[data-autofocus]");
    (
      preferred ?? panelRef.current?.querySelector<HTMLElement>(FOCUSABLE)
    )?.focus();
    const keydown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") cancelRef.current();
    };
    document.addEventListener("keydown", keydown);
    return () => {
      document.removeEventListener("keydown", keydown);
      previouslyFocused?.focus();
    };
  }, [panelRef]);
}

function ModalFrame({
  panelRef,
  title,
  onMouseDown,
  onKeyDown,
  children,
}: {
  panelRef: RefObject<HTMLDivElement | null>;
  title: string;
  onMouseDown: (e: MouseEvent<HTMLDivElement>) => void;
  onKeyDown: (e: KeyboardEvent) => void;
  children: ReactNode;
}) {
  return (
    <div
      className="mcp-install-backdrop"
      role="presentation"
      onMouseDown={onMouseDown}
      onKeyDown={onKeyDown}
    >
      <div
        ref={panelRef}
        className="mcp-install-panel card form-grid"
        role="dialog"
        aria-modal="true"
        aria-label={title}
      >
        {children}
      </div>
    </div>
  );
}

export function ConfirmModal({
  title,
  message,
  confirmLabel,
  cancelLabel,
  tone = "default",
  onConfirm,
  onCancel,
}: {
  title: string;
  message: string;
  /** Explicit action label ("削除", "移動"…). "確認" is too generic for a
   *  destructive dialog and gives no hint about what Enter will do. */
  confirmLabel?: string;
  cancelLabel?: string;
  tone?: "default" | "danger";
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef(onCancel);
  cancelRef.current = onCancel;
  useModalDialog(panelRef, () => cancelRef.current());
  return (
    <ModalFrame
      panelRef={panelRef}
      title={title}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onCancel();
      }}
      onKeyDown={(e) => {
        if (e.key === "Tab") trapFocus(panelRef.current as HTMLElement, e);
      }}
    >
      <h2>{title}</h2>
      <p>{message}</p>
      <div className="form-actions">
        <Button
          variant={tone === "danger" ? "destructive" : "default"}
          onClick={() => {
            onConfirm();
            onCancel();
          }}
        >
          {confirmLabel ?? t(ja.approve)}
        </Button>
        <Button data-autofocus variant="ghost" type="button" onClick={onCancel}>
          {cancelLabel ?? t(ja.cancel)}
        </Button>
      </div>
    </ModalFrame>
  );
}

/** Button that owns its destructive confirmation, so a call site cannot forget
 *  the dialog (the old inline `if (!confirm(...))` pattern). */
export function ConfirmButton({
  title,
  message,
  confirmLabel,
  tone = "danger",
  onConfirm,
  children,
  ...props
}: ComponentProps<typeof Button> & {
  title: string;
  message: string;
  confirmLabel?: string;
  tone?: "default" | "danger";
  onConfirm: () => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button {...props} onClick={() => setOpen(true)}>
        {children}
      </Button>
      {open && (
        <ConfirmModal
          title={title}
          message={message}
          confirmLabel={confirmLabel}
          tone={tone}
          onConfirm={onConfirm}
          onCancel={() => setOpen(false)}
        />
      )}
    </>
  );
}

export function PromptModal({
  title,
  defaultValue = "",
  onConfirm,
  onCancel,
}: {
  title: string;
  defaultValue?: string;
  onConfirm: (value: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(defaultValue);
  const panelRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const cancelRef = useRef(onCancel);
  cancelRef.current = onCancel;
  useModalDialog(panelRef, () => cancelRef.current());
  return (
    <ModalFrame
      panelRef={panelRef}
      title={title}
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onCancel();
      }}
      onKeyDown={(e) => {
        if (e.key === "Tab") trapFocus(panelRef.current as HTMLElement, e);
      }}
    >
      <form
        onSubmit={(e: FormEvent) => {
          e.preventDefault();
          onConfirm(value);
        }}
      >
        <h2>{title}</h2>
        <input
          ref={inputRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          data-autofocus
        />
        <div className="form-actions">
          <Button type="submit">{t(ja.confirm)}</Button>
          <Button variant="ghost" type="button" onClick={onCancel}>
            {t(ja.cancel)}
          </Button>
        </div>
      </form>
    </ModalFrame>
  );
}
