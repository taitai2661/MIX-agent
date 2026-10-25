/// Typed prompt-insert channel: welcome chips pre-fill the composer.
///
/// This used to be a `window` CustomEvent (`mix:insert-prompt`) dispatched
/// from the transcript and listened to in the composer — an ad-hoc event bus
/// with an untyped `detail` and no delivery guarantee across mount order. A
/// tiny zustand store carries the same signal: the payload is typed, late
/// subscribers still see the latest request via the monotonically increasing
/// `nonce`, and it works without a DOM.

import { create } from "zustand";

export type PromptMode = "chat" | "thinking" | "agent";

export interface PromptInsertRequest {
  text: string;
  mode?: PromptMode;
  /** Whether to append text to current draft or replace it (default: false = replace). */
  append?: boolean;
  /** Incremented per request so identical payloads still notify. */
  nonce: number;
}

interface PromptInsertState {
  request: PromptInsertRequest | null;
  insertPrompt: (text: string, mode?: PromptMode) => void;
  appendPrompt: (text: string) => void;
  insertFileReference: (path: string) => void;
}

export const usePromptInsert = create<PromptInsertState>((set) => ({
  request: null,
  insertPrompt: (text, mode) =>
    set((s) => ({
      request: {
        text,
        mode,
        append: false,
        nonce: (s.request?.nonce ?? 0) + 1,
      },
    })),
  appendPrompt: (text) =>
    set((s) => ({
      request: { text, append: true, nonce: (s.request?.nonce ?? 0) + 1 },
    })),
  insertFileReference: (path) =>
    set((s) => ({
      request: {
        text: `@${path} `,
        append: true,
        nonce: (s.request?.nonce ?? 0) + 1,
      },
    })),
}));

/** Subscribe to prompt-insert requests. Returns an unsubscribe function. */
export function subscribePromptInsert(
  listener: (request: PromptInsertRequest) => void,
): () => void {
  let lastNonce = usePromptInsert.getState().request?.nonce ?? 0;
  return usePromptInsert.subscribe((s) => {
    const request = s.request;
    if (request && request.nonce !== lastNonce) {
      lastNonce = request.nonce;
      listener(request);
    }
  });
}
