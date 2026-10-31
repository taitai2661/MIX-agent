import { api, type Row } from "@/app/api";
import { t } from "@/app/i18n";
import { uuid } from "@/app/uuid";
import type { QueryClient } from "@tanstack/react-query";
import {
  useRef,
  type Dispatch,
  type FormEvent,
  type SetStateAction,
} from "react";

import type { Artifact, ChatMode, Feedback, SendMessage } from "./types";

export type ChatActionsOptions = {
  conversationId?: string;
  navigate: (to: string) => void;
  queryClient: QueryClient;
  text: string;
  attachments: Artifact[];
  busy: boolean;
  active: boolean;
  uploading: boolean;
  needsProviderSetup: boolean;
  hasNoProviders: boolean;
  model: string;
  mode: ChatMode;
  researchMode: boolean;
  agent: string;
  temporaryMode: boolean;
  allowTools: boolean;
  selectedProject: string;
  editText: string;
  savingEdit: boolean;
  lastUserMessage?: Row;
  setText: Dispatch<SetStateAction<string>>;
  setAttachments: Dispatch<SetStateAction<Artifact[]>>;
  setBusy: Dispatch<SetStateAction<boolean>>;
  setError: Dispatch<SetStateAction<unknown>>;
  setSendFailed: Dispatch<SetStateAction<boolean>>;
  setRunId: Dispatch<SetStateAction<string>>;
  setUploading: Dispatch<SetStateAction<boolean>>;
  setEditingId: Dispatch<SetStateAction<string | null>>;
  setEditText: Dispatch<SetStateAction<string>>;
  setSavingEdit: Dispatch<SetStateAction<boolean>>;
};

export type ChatActions = {
  send: (event?: FormEvent<HTMLFormElement>) => Promise<void>;
  attach: (files: File[]) => Promise<void>;
  rate: (
    messageId: string,
    current: Feedback,
    value: Exclude<Feedback, null>,
  ) => Promise<void>;
  startEdit: (message: Row) => void;
  cancelEdit: () => void;
  saveEdit: (message: Row) => Promise<void>;
  retryLastRun: () => Promise<void>;
};

/**
 * Composer/transcript mutations: send, upload, feedback, and inline-edit
 * regeneration. All of them share the same conversation selection, so they are
 * kept together instead of being scattered through the render component.
 */
export function useChatActions({
  conversationId,
  navigate,
  queryClient,
  text,
  attachments,
  busy,
  active,
  uploading,
  needsProviderSetup,
  hasNoProviders,
  model,
  mode,
  researchMode,
  agent,
  temporaryMode,
  allowTools,
  selectedProject,
  editText,
  savingEdit,
  lastUserMessage,
  setText,
  setAttachments,
  setBusy,
  setError,
  setSendFailed,
  setRunId,
  setUploading,
  setEditingId,
  setEditText,
  setSavingEdit,
}: ChatActionsOptions): ChatActions {
  const pendingSend = useRef<{
    signature: string;
    key: string;
    conversation: string | undefined;
  } | null>(null);

  async function send(e?: FormEvent<HTMLFormElement>) {
    e?.preventDefault();
    if ((!text.trim() && !attachments.length) || busy || active || uploading)
      return;
    if (needsProviderSetup) {
      setError(
        new Error(
          t(
            hasNoProviders
              ? "先にAI Providerを接続してください。"
              : "先にモデルを取得してください。",
          ),
        ),
      );
      return;
    }
    setError(null);
    setSendFailed(false);
    setBusy(true);
    try {
      const body = {
        content: text,
        model_id: model,
        mode: researchMode ? "agent" : mode,
        agent_id: agent,
        artifact_ids: attachments.map((a) => a.artifact_id),
        temporary_mode: temporaryMode,
        allow_tools: allowTools,
        research_mode: researchMode && !temporaryMode,
      };
      const signature = JSON.stringify({ id: conversationId, ...body });
      if (!pendingSend.current || pendingSend.current.signature !== signature) {
        pendingSend.current = {
          signature,
          key: uuid(),
          conversation: conversationId,
        };
      }
      let conversation = pendingSend.current.conversation;
      if (!conversation) {
        const c = await api<Row>("/conversations", "POST", {
          project_id: temporaryMode ? null : selectedProject || null,
        });
        conversation = c.id;
        pendingSend.current.conversation = c.id;
      }
      const r = await api<SendMessage>(
        "/conversations/" + conversation + "/messages",
        "POST",
        body,
        pendingSend.current.key,
        { retryUnsafe: true },
      );
      pendingSend.current = null;
      setText("");
      setAttachments([]);
      await queryClient.invalidateQueries({ queryKey: ["/conversations"] });
      if (conversationId !== conversation) navigate("/chat/" + conversation);
      else {
        setRunId(r.run_id);
        void queryClient.invalidateQueries({
          queryKey: ["messages", conversationId],
        });
      }
    } catch (e) {
      // The error box gets a resend action: clearing an error without
      // performing the retry made "再試行" look like it had sent the message.
      setSendFailed(true);
      setError(e);
    } finally {
      setBusy(false);
    }
  }

  async function attach(files: File[]) {
    if (!files.length) return;
    setError(null);
    setUploading(true);
    const failed: string[] = [];
    try {
      for (const file of files) {
        if (file.size > 20 * 1024 * 1024) {
          failed.push(`${file.name}: ${t("20MBを超えています")}`);
          continue;
        }
        try {
          const f = new FormData();
          f.set("file", file);
          const a = await api<Artifact>("/artifacts", "POST", f);
          setAttachments((xs) => [...xs, a]);
        } catch (cause) {
          failed.push(
            `${file.name}: ${cause instanceof Error ? cause.message : String(cause)}`,
          );
        }
      }
      if (failed.length) {
        setError(new Error(failed.join("\n")));
      }
    } finally {
      setUploading(false);
    }
  }

  async function rate(
    messageId: string,
    current: Feedback,
    value: Exclude<Feedback, null>,
  ) {
    try {
      await api("/messages/" + messageId + "/feedback", "PUT", {
        value: current === value ? null : value,
      });
      void queryClient.invalidateQueries({
        queryKey: ["messages", conversationId],
      });
    } catch (e) {
      setError(e);
    }
  }

  function startEdit(m: Row) {
    setEditingId(m.id);
    setEditText(String(m.data.content || ""));
    setError(null);
  }

  function cancelEdit() {
    setEditingId(null);
    setEditText("");
  }

  /** Replace a sent user turn and regenerate from it. The backend drops that
   *  turn and everything after it, so the reply stream starts clean. */
  async function regenerate(m: Row, contentOverride?: string) {
    const content = (contentOverride ?? editText).trim();
    if (!content || !conversationId || savingEdit || active || busy) return;
    setSavingEdit(true);
    setError(null);
    try {
      const body = {
        content,
        model_id: model,
        mode: researchMode ? "agent" : mode,
        agent_id: agent,
        artifact_ids: (m.data.artifact_ids as string[]) || [],
        temporary_mode: temporaryMode,
        allow_tools: allowTools,
        research_mode: researchMode && !temporaryMode,
      };
      const r = await api<SendMessage>(
        "/conversations/" + conversationId + "/messages/" + m.id + "/edit",
        "POST",
        body,
        uuid(),
        { retryUnsafe: true },
      );
      cancelEdit();
      setRunId(r.run_id);
      void queryClient.invalidateQueries({
        queryKey: ["messages", conversationId],
      });
      void queryClient.invalidateQueries({
        queryKey: ["tool-calls", conversationId],
      });
      void queryClient.invalidateQueries({ queryKey: ["/conversations"] });
    } catch (e) {
      setError(e);
    } finally {
      setSavingEdit(false);
    }
  }

  const saveEdit = (m: Row) => regenerate(m);

  async function retryLastRun() {
    if (!lastUserMessage) return;
    await regenerate(
      lastUserMessage,
      String(lastUserMessage.data.content || ""),
    );
  }

  return {
    send,
    attach,
    rate,
    startEdit,
    cancelEdit,
    saveEdit,
    retryLastRun,
  };
}
