import {
  createContext,
  useCallback,
  useContext,
  useState,
  type ReactNode,
} from "react";

import { ConfirmModal } from "./modal";

export type ConfirmOptions = {
  title: string;
  message?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  tone?: "default" | "danger";
};

type Pending = {
  options: ConfirmOptions;
  resolve: (value: boolean) => void;
};

const ConfirmContext = createContext<
  ((options: ConfirmOptions | string) => Promise<boolean>) | null
>(null);

/**
 * Promise-based confirmation dialog. Replaces the native `window.confirm`
 * calls so destructive actions use the app's own modal, wording and styling.
 */
export function ConfirmProvider({ children }: { children: ReactNode }) {
  const [pending, setPending] = useState<Pending | null>(null);
  const confirm = useCallback(
    (input: ConfirmOptions | string) =>
      new Promise<boolean>((resolve) => {
        setPending({
          options: typeof input === "string" ? { title: input } : input,
          resolve,
        });
      }),
    [],
  );

  return (
    <ConfirmContext.Provider value={confirm}>
      {children}
      {pending && (
        <ConfirmModal
          title={pending.options.title}
          message={pending.options.message ?? ""}
          confirmLabel={pending.options.confirmLabel}
          cancelLabel={pending.options.cancelLabel}
          tone={pending.options.tone}
          onConfirm={() => {
            pending.resolve(true);
            setPending(null);
          }}
          onCancel={() => {
            pending.resolve(false);
            setPending(null);
          }}
        />
      )}
    </ConfirmContext.Provider>
  );
}

export function useConfirm() {
  const confirm = useContext(ConfirmContext);
  const fallback = useCallback((input: ConfirmOptions | string) => {
    const options = typeof input === "string" ? { title: input } : input;
    const text = [options.title, options.message].filter(Boolean).join("\n\n");
    return Promise.resolve(
      typeof window === "undefined" ? false : window.confirm(text),
    );
  }, []);
  return confirm ?? fallback;
}
