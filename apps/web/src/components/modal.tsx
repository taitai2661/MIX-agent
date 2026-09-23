import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent, type MouseEvent, type ReactNode, type RefObject } from "react";
import { Button } from "@/components/button";
import { ja } from "@/app/strings";

const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

function trapFocus(panel: HTMLElement, event: KeyboardEvent) {
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

function useModalDialog(panelRef: RefObject<HTMLDivElement | null>, onCancel: () => void) {
  useEffect(() => {
    const previouslyFocused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    panelRef.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    const keydown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") onCancel();
    };
    document.addEventListener("keydown", keydown);
    return () => {
      document.removeEventListener("keydown", keydown);
      previouslyFocused?.focus();
    };
  }, [onCancel, panelRef]);
}

function ModalFrame({ panelRef, title, onMouseDown, onKeyDown, children }: {
  panelRef: RefObject<HTMLDivElement | null>;
  title: string;
  onMouseDown: (e: MouseEvent<HTMLDivElement>) => void;
  onKeyDown: (e: KeyboardEvent) => void;
  children: ReactNode;
}) {
  return (
    <div
      className="mcp-install-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onMouseDown={onMouseDown}
    >
      <div ref={panelRef} className="mcp-install-panel card form-grid" onKeyDown={onKeyDown}>
        {children}
      </div>
    </div>
  );
}

export function ConfirmModal({
  title,
  message,
  onConfirm,
  onCancel,
}: {
  title: string;
  message: string;
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
          autoFocus
          onClick={() => {
            onConfirm();
            onCancel();
          }}
        >
          {ja.approve}
        </Button>
        <Button variant="ghost" type="button" onClick={onCancel}>
          {ja.cancel}
        </Button>
      </div>
    </ModalFrame>
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
          autoFocus
        />
        <div className="form-actions">
          <Button type="submit">{ja.confirm}</Button>
          <Button variant="ghost" type="button" onClick={onCancel}>
            {ja.cancel}
          </Button>
        </div>
      </form>
    </ModalFrame>
  );
}