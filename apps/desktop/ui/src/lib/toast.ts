import { create } from "zustand";

export interface ToastAction {
  label: string;
  onClick: () => void;
}

export interface ToastItem {
  id: string;
  /** Incrementing generation: a re-push with the same `id` must remount the row
   * so its timer and hover-pause state restart instead of being inherited. */
  seq: number;
  message: string;
  action?: ToastAction;
  /** How long the toast stays up; also the window for an Undo action. */
  timeoutMs: number;
  /**
   * Runs when the toast goes away without its action being taken (timeout or
   * the close button). Undoable deletes use this as the single timer, so the
   * delete can no longer fire while the "Undo" button is still visible.
   */
  onDismiss?: () => void;
}

interface ToastState {
  toasts: ToastItem[];
  push: (input: {
    message: string;
    action?: ToastAction;
    timeoutMs?: number;
    id?: string;
    onDismiss?: () => void;
  }) => string;
  dismiss: (id: string, options?: { runOnDismiss?: boolean }) => void;
}

let counter = 0;

/**
 * Tiny app-level toast queue.
 *
 * Used for short-lived, actionable confirmations ("delete thread → undo",
 * "settings saved → undo"). Deliberately not a general notification system: no
 * stacking limit and no persistence. The `Toaster` component owns the timer so
 * unmounted tests never leave a pending `setTimeout`.
 */
export const useToasts = create<ToastState>((set, get) => ({
  toasts: [],
  push: ({ message, action, timeoutMs = 7000, id, onDismiss }) => {
    counter += 1;
    const seq = counter;
    const toastId = id ?? `toast-${seq}`;
    // A fixed id can be re-pushed (settings autosave, retry). The row it
    // replaces may own deferred work through `onDismiss`; run it rather than
    // dropping it silently when the id is reused.
    const previous = get().toasts.find((toast) => toast.id === toastId);
    if (previous?.onDismiss) {
      try {
        previous.onDismiss();
      } catch (error) {
        console.error("toast onDismiss failed", error);
      }
    }
    set((state) => ({
      toasts: [
        ...state.toasts.filter((toast) => toast.id !== toastId),
        { id: toastId, seq, message, action, timeoutMs, onDismiss },
      ],
    }));
    return toastId;
  },
  dismiss: (id, options) => {
    const toast = get().toasts.find((item) => item.id === id);
    set((state) => ({
      toasts: state.toasts.filter((item) => item.id !== id),
    }));
    // `runOnDismiss: false` is used after the action ran (Undo). The action is
    // the promise that the deferred work should not happen.
    if (toast && options?.runOnDismiss !== false) {
      try {
        toast.onDismiss?.();
      } catch (error) {
        console.error("toast onDismiss failed", error);
      }
    }
  },
}));
