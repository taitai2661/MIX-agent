import { createContext, useContext, type ReactNode } from "react";
import type { Row } from "@/app/api";
import type { ApprovalDecisionHandler, Feedback } from "./types";

/**
 * Chat callbacks used by the transcript (and, via the provider, by any nested
 * panel). Bundling them keeps Chat.tsx from drilling the same eleven handlers
 * through three layers as individual props.
 */
export type ChatActions = {
  onEditChange: (value: string) => void;
  onEditCancel: () => void;
  onEditSave: (message: Row) => void;
  onEditStart: (message: Row) => void;
  onRate: (
    messageId: string,
    current: Feedback,
    value: Exclude<Feedback, null>,
  ) => void;
  onApproval: ApprovalDecisionHandler;
  onBrowserChange: (browserRunId: string) => void;
  onRetryRun: () => void;
  onError: (error: unknown) => void;
  onResumed: () => void;
  onBudgetResolved: () => void;
};

const noop = () => {};

/** Used when a transcript/message is rendered without a surrounding provider
 *  (unit tests, isolated panels). Kept here so every consumer shares one stable
 *  identity instead of each creating its own fallback object. */
export const FALLBACK_ACTIONS: ChatActions = {
  onEditChange: noop,
  onEditCancel: noop,
  onEditSave: noop,
  onEditStart: noop,
  onRate: noop,
  onApproval: noop,
  onBrowserChange: noop,
  onRetryRun: noop,
  onError: noop,
  onResumed: noop,
  onBudgetResolved: noop,
};

const ChatActionsContext = createContext<ChatActions | null>(null);

export function ChatActionsProvider({
  actions,
  children,
}: {
  actions: ChatActions;
  children: ReactNode;
}) {
  return (
    <ChatActionsContext.Provider value={actions}>
      {children}
    </ChatActionsContext.Provider>
  );
}

export function useChatActionsContext(): ChatActions | null {
  return useContext(ChatActionsContext);
}
