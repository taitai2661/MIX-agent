/// Local state for the composer: draft persistence, attachments, the
/// mention/slash popovers, ArrowUp history and the keyboard contract.
///
/// The component keeps layout and model-derived copy; everything that mutates
/// the draft or reads the textarea selection lives here so `Composer.tsx` stays
/// a composition of the toolbar, attachment row and textarea.

import { useCallback, useEffect, useRef, useState } from "react";
import type {
  Dispatch,
  KeyboardEvent as ReactKeyboardEvent,
  RefObject,
  SetStateAction,
} from "react";
import {
  clearComposerDraft,
  loadComposerDraft,
  saveComposerDraft,
} from "@/lib/drafts";
import { t } from "@/lib/i18n";
import { loadPromptHistory, pushPromptHistory } from "@/lib/promptHistory";
import { subscribePromptInsert } from "@/lib/promptInsert";
import type { SendShortcut } from "@/lib/ui";
import type { ModelView } from "@/lib/types";
import { ALL_MODES } from "./composerModes";
import type {
  ComposerMode,
  ComposerProps,
  DraftAttachment,
} from "./composerTypes";
import type { SlashCommand } from "./SlashCommandPopover";
import { useAttachments } from "./useAttachments";
import { useVoiceInput } from "./useVoiceInput";

interface UseComposerStateArgs {
  setMode: (mode: ComposerMode) => void;
  allowedModes?: ReadonlyArray<ComposerMode>;
  draftKey: string | null;
  /** Root of the active project; enables the `@` mention popover. */
  rootPath: string | null;
  sendShortcut: SendShortcut;
  busy: boolean;
  modelId: string;
  selectedModel: ModelView | undefined;
  staleModel: boolean;
  onSend: ComposerProps["onSend"];
  onSteer?: ComposerProps["onSteer"];
}

export interface ComposerState {
  draft: string;
  setDraft: Dispatch<SetStateAction<string>>;
  /** Typing resets ArrowUp history recall without touching the stored draft. */
  onDraftChange: (value: string) => void;
  onCompositionStart: () => void;
  onCompositionEnd: () => void;
  attachments: DraftAttachment[];
  attachError: string | null;
  attaching: boolean;
  addImageFiles: (files: Iterable<File>) => Promise<void>;
  onAttach: () => Promise<void>;
  removeAttachment: (index: number) => void;
  temporaryMode: boolean;
  setTemporaryMode: (value: boolean) => void;
  allowTools: boolean;
  setAllowTools: (value: boolean) => void;
  isDragging: boolean;
  setIsDragging: Dispatch<SetStateAction<boolean>>;
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  mentionQuery: string | null;
  mentionIndex: number;
  setMentionQuery: (query: string | null) => void;
  setMentionIndex: Dispatch<SetStateAction<number>>;
  slashQuery: string | null;
  slashIndex: number;
  setSlashQuery: (query: string | null) => void;
  setSlashIndex: Dispatch<SetStateAction<number>>;
  checkMention: () => void;
  checkSlash: () => void;
  onSelectMention: (path: string) => void;
  onSelectSlashCommand: (command: SlashCommand) => void;
  onInsertMention: () => void;
  handleDropFiles: (files: File[]) => Promise<void>;
  onSubmit: () => Promise<void>;
  onKeyDown: (event: ReactKeyboardEvent<HTMLTextAreaElement>) => void;
  hasImage: boolean;
  visionWarning: boolean;
  sendDisabledReason: string | null;
  canSend: boolean;
  listening: boolean;
  toggleVoiceInput: () => void;
  speechSupported: boolean;
}

export function useComposerState({
  setMode,
  allowedModes,
  draftKey,
  rootPath,
  sendShortcut,
  busy,
  modelId,
  selectedModel,
  staleModel,
  onSend,
  onSteer,
}: UseComposerStateArgs): ComposerState {
  const [draft, setDraft] = useState(() => loadComposerDraft(draftKey));
  const {
    attachments,
    attachError,
    attaching,
    addImageFiles,
    onAttach,
    removeAttachment,
    clearAttachments,
  } = useAttachments();
  const { listening, toggleVoiceInput, speechSupported } = useVoiceInput(
    draft,
    setDraft,
  );
  const [temporaryMode, setTemporaryMode] = useState(false);
  const [allowTools, setAllowTools] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const isComposingRef = useRef(false);
  const compositionEndTimestampRef = useRef(0);
  const draftKeyRef = useRef<string | null>(draftKey);
  const draftRef = useRef(draft);
  draftRef.current = draft;

  const historyIndexRef = useRef<number>(-1);
  const historyDraftRef = useRef<string>("");

  const onDraftChange = (value: string) => {
    setDraft(value);
    historyIndexRef.current = -1;
  };

  const onCompositionStart = () => {
    isComposingRef.current = true;
  };

  const onCompositionEnd = () => {
    isComposingRef.current = false;
    compositionEndTimestampRef.current = Date.now();
  };

  const [mentionQuery, setMentionQuery] = useState<string | null>(null);
  const [mentionIndex, setMentionIndex] = useState(0);

  const checkMention = useCallback(() => {
    const el = textareaRef.current;
    if (!el || !rootPath) {
      setMentionQuery(null);
      return;
    }
    const pos = el.selectionStart;
    const textBefore = el.value.slice(0, pos);
    const match = textBefore.match(/(?:^|\s)@([a-zA-Z0-9_\-./]*)$/);
    if (match) {
      setMentionQuery(match[1]);
    } else {
      setMentionQuery(null);
    }
  }, [rootPath]);

  const onSelectMention = useCallback((path: string) => {
    const el = textareaRef.current;
    if (!el) return;
    const pos = el.selectionStart;
    const textBefore = el.value.slice(0, pos);
    const textAfter = el.value.slice(pos);
    const match = textBefore.match(/(?:^|\s)@([a-zA-Z0-9_\-./]*)$/);
    if (!match) return;

    const replaceStart = pos - match[1].length - 1;
    const newText = el.value.slice(0, replaceStart) + `@${path} ` + textAfter;
    setDraft(newText);
    setMentionQuery(null);

    requestAnimationFrame(() => {
      el.focus();
      const nextPos = replaceStart + path.length + 2;
      el.setSelectionRange(nextPos, nextPos);
    });
  }, []);

  const [slashQuery, setSlashQuery] = useState<string | null>(null);
  const [slashIndex, setSlashIndex] = useState(0);

  const checkSlash = useCallback(() => {
    const el = textareaRef.current;
    if (!el) {
      setSlashQuery(null);
      return;
    }
    const pos = el.selectionStart;
    const textBefore = el.value.slice(0, pos);
    const match = textBefore.match(/(?:^|\s)\/([a-zA-Z0-9_-]*)$/);
    if (match) {
      setSlashQuery(match[1]);
    } else {
      setSlashQuery(null);
    }
  }, []);

  const onSelectSlashCommand = useCallback((command: SlashCommand) => {
    const el = textareaRef.current;
    if (!el) return;
    const pos = el.selectionStart;
    const textBefore = el.value.slice(0, pos);
    const textAfter = el.value.slice(pos);
    const match = textBefore.match(/(?:^|\s)\/([a-zA-Z0-9_-]*)$/);
    if (!match) return;

    const replaceStart = pos - match[1].length - 1;
    const newText =
      el.value.slice(0, replaceStart) + command.prompt + textAfter;
    setDraft(newText);
    setSlashQuery(null);

    requestAnimationFrame(() => {
      el.focus();
      const nextPos = replaceStart + command.prompt.length;
      el.setSelectionRange(nextPos, nextPos);
    });
  }, []);

  const onInsertMention = () => {
    const el = textareaRef.current;
    if (!el || !rootPath) return;
    const pos = el.selectionStart;
    const textBefore = el.value.slice(0, pos);
    const textAfter = el.value.slice(pos);
    const prefix =
      textBefore.length > 0 && !textBefore.endsWith(" ") ? " @" : "@";
    setDraft(textBefore + prefix + textAfter);
    setMentionQuery("");
    requestAnimationFrame(() => {
      el.focus();
      const nextPos = pos + prefix.length;
      el.setSelectionRange(nextPos, nextPos);
    });
  };

  const handleDropFiles = useCallback(
    async (files: File[]) => {
      const images = files.filter((f) => f.type.startsWith("image/"));
      const nonImages = files.filter((f) => !f.type.startsWith("image/"));
      if (images.length > 0) {
        await addImageFiles(images);
      }
      if (nonImages.length > 0) {
        for (const file of nonImages) {
          if (file.size <= 128 * 1024) {
            try {
              const text = await file.text();
              setDraft((prev) => {
                const prefix = prev ? prev + "\n\n" : "";
                return `${prefix}# ${file.name}\n\`\`\`\n${text}\n\`\`\``;
              });
            } catch {
              setDraft((prev) =>
                prev ? `${prev} @${file.name}` : `@${file.name} `,
              );
            }
          } else {
            setDraft((prev) =>
              prev ? `${prev} @${file.name}` : `@${file.name} `,
            );
          }
        }
      }
    },
    [addImageFiles],
  );

  // Draft persistence: one slot per conversation, via `lib/drafts.ts`
  // (best-effort, timestamped, expires after 30 days). Switching threads used
  // to either drop the draft or leak it into the next thread. The ref dance
  // avoids persisting the freshly-restored text of the new thread back into the
  // old thread's slot.
  useEffect(() => {
    const key = draftKey;
    if (key !== draftKeyRef.current) {
      const prev = draftKeyRef.current;
      const current = draftRef.current;
      // Persist the old thread's text outside the state updater (updaters
      // must be pure; side-effects here double-fire under StrictMode).
      if (prev !== undefined) {
        if (current) saveComposerDraft(prev, current);
        else clearComposerDraft(prev);
      }
      draftKeyRef.current = key;
      setDraft(loadComposerDraft(key));
      // Attachments and the per-send switches are not part of the saved draft.
      // Carrying them across a thread switch silently included one task's
      // screenshot (or an armed temporary mode) in the next task's message.
      clearAttachments();
      setTemporaryMode(false);
      setAllowTools(false);
    }
    // Deliberately keyed on the draft slot only: the helpers are recreated per
    // render, and re-running this effect would wipe the current draft.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draftKey]);

  useEffect(() => {
    const key = draftKeyRef.current;
    // Keystrokes fire per character; debounce the write so typing doesn't
    // hit storage (a synchronous disk read) on every frame.
    const timer = window.setTimeout(() => {
      if (draft) saveComposerDraft(key, draft);
      else clearComposerDraft(key);
    }, 300);
    return () => window.clearTimeout(timer);
  }, [draft]);

  // Focus the box on first mount so the user can type immediately.
  useEffect(() => {
    const id = window.requestAnimationFrame(() => textareaRef.current?.focus());
    return () => window.cancelAnimationFrame(id);
  }, []);

  // Grow the textarea with its content, bounded by the `max-h-56` class.
  // Written as a `field-sizing` opt-out rather than a pixel write: setting
  // `style.height` fought the `min-h-16`/`max-h-56` classes, which is how the
  // box ended up taller than its cap on long drafts.
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    const next = Math.min(el.scrollHeight, 224);
    el.style.height = `${next}px`;
  }, [draft]);

  // Welcome chips (or external callers) pre-fill or append to the draft through
  // the typed prompt-insert store instead of threading state through ChatPane.
  useEffect(() => {
    const onInsert = (request: {
      text: string;
      mode?: ComposerMode;
      append?: boolean;
    }) => {
      if (typeof request?.text === "string") {
        if (request.append) {
          setDraft((prev) => {
            const trimmed = prev.trimEnd();
            return trimmed ? `${trimmed} ${request.text}` : request.text;
          });
        } else {
          setDraft(request.text);
        }
      }
      // A greeting picked on one surface may name a mode the current surface
      // does not expose (e.g. "thinking" after switching to Desktop-Agent), so
      // only apply a mode that is actually allowed here.
      if (request?.mode) {
        const allowed =
          allowedModes && allowedModes.length > 0
            ? allowedModes
            : ALL_MODES.map((m) => m.id);
        if ((allowed as readonly string[]).includes(request.mode))
          setMode(request.mode);
      }
      window.requestAnimationFrame(() => textareaRef.current?.focus());
    };
    return subscribePromptInsert(onInsert);
  }, [setMode, allowedModes]);

  const isAutoModel = modelId === "auto";
  const hasImage = attachments.some((a) => a.mime_type.startsWith("image/"));
  // Auto resolves to a vision-capable model when an image is attached, so the
  // warning only applies to an explicitly picked model without vision.
  const visionWarning =
    hasImage &&
    !isAutoModel &&
    selectedModel != null &&
    !selectedModel.supports_vision;
  const sendDisabledReason = busy
    ? t("実行中です")
    : staleModel
      ? t("選択したモデルが利用できません")
      : visionWarning
        ? t("画像を添付するにはVision対応モデルを選択してください")
        : draft.trim().length === 0
          ? t("メッセージを入力してください")
          : !modelId
            ? t("モデルを選択してください")
            : null;
  const canSend =
    draft.trim().length > 0 &&
    !busy &&
    !!modelId &&
    !staleModel &&
    (!hasImage ||
      isAutoModel ||
      !selectedModel ||
      selectedModel.supports_vision);

  const submit = async () => {
    // While a Run is working, Enter sends the text as an interjection instead
    // of being ignored. The draft is only cleared once the backend accepted it.
    if (busy) {
      const trimmed = draft.trim();
      if (!trimmed || !onSteer) return;
      try {
        await onSteer(trimmed);
      } catch {
        textareaRef.current?.focus();
        return;
      }
      pushPromptHistory(trimmed);
      historyIndexRef.current = -1;
      historyDraftRef.current = "";
      setDraft("");
      clearComposerDraft(draftKeyRef.current);
      textareaRef.current?.focus();
      return;
    }
    if (!canSend) return;
    const trimmed = draft.trim();
    const pending = onSend(trimmed, attachments, { temporaryMode, allowTools });
    if (pending && typeof (pending as Promise<unknown>).then === "function") {
      try {
        await pending;
      } catch {
        // Keep the prompt and its attachments so the user can retry. The error is
        // surfaced by the mutation's ErrorBox above the composer.
        textareaRef.current?.focus();
        return;
      }
    }
    // A temporary turn must not leave the prompt in the local ArrowUp history.
    // The same promise ("この送信は保存されません") applies here, not only to
    // the database transcript.
    if (!temporaryMode) pushPromptHistory(trimmed);
    historyIndexRef.current = -1;
    historyDraftRef.current = "";
    setDraft("");
    clearAttachments();
    clearComposerDraft(draftKeyRef.current);
    // Temporary mode is per-send: leaving it on would silently discard the next
    // turn's transcript too.
    setTemporaryMode(false);
    setAllowTools(false);
    textareaRef.current?.focus();
  };

  const onKeyDown = (e: ReactKeyboardEvent<HTMLTextAreaElement>) => {
    const isImeComposing =
      e.nativeEvent.isComposing ||
      isComposingRef.current ||
      e.keyCode === 229 ||
      Date.now() - compositionEndTimestampRef.current < 60;

    if (mentionQuery !== null) {
      if (e.key === "Escape") {
        e.preventDefault();
        setMentionQuery(null);
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setMentionIndex((i) => i + 1);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setMentionIndex((i) => Math.max(0, i - 1));
        return;
      }
      if (
        (e.key === "Enter" || e.key === "Tab") &&
        !e.shiftKey &&
        !isImeComposing
      ) {
        const items = document.querySelectorAll("[data-mention-item]");
        const active = items[mentionIndex] as HTMLElement | undefined;
        if (active) {
          e.preventDefault();
          active.click();
          return;
        }
      }
    }

    if (slashQuery !== null && mentionQuery === null) {
      if (e.key === "Escape") {
        e.preventDefault();
        setSlashQuery(null);
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSlashIndex((i) => i + 1);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSlashIndex((i) => Math.max(0, i - 1));
        return;
      }
      if (
        (e.key === "Enter" || e.key === "Tab") &&
        !e.shiftKey &&
        !isImeComposing
      ) {
        const items = document.querySelectorAll("[data-slash-item]");
        const active = items[slashIndex] as HTMLElement | undefined;
        if (active) {
          e.preventDefault();
          active.click();
          return;
        }
      }
    }

    if (mentionQuery === null && slashQuery === null && !isImeComposing) {
      if (e.key === "ArrowUp") {
        const el = textareaRef.current;
        const inHistory = historyIndexRef.current >= 0;
        const atStart =
          inHistory ||
          !el ||
          (el.selectionStart === 0 && el.selectionEnd === 0);
        if (atStart) {
          const history = loadPromptHistory();
          if (history.length > 0) {
            const nextIndex = Math.min(
              historyIndexRef.current + 1,
              history.length - 1,
            );
            if (nextIndex !== historyIndexRef.current) {
              if (historyIndexRef.current === -1) {
                historyDraftRef.current = draft;
              }
              historyIndexRef.current = nextIndex;
              e.preventDefault();
              setDraft(history[nextIndex]);
            }
          }
        }
      } else if (e.key === "ArrowDown" && historyIndexRef.current >= 0) {
        const nextIndex = historyIndexRef.current - 1;
        e.preventDefault();
        if (nextIndex >= 0) {
          const history = loadPromptHistory();
          historyIndexRef.current = nextIndex;
          setDraft(history[nextIndex]);
        } else {
          historyIndexRef.current = -1;
          setDraft(historyDraftRef.current);
        }
      }
    }

    if (e.key === "Enter" && !isImeComposing) {
      if (sendShortcut === "modifier") {
        // Modifier mode: Cmd+Enter or Ctrl+Enter sends; regular Enter inserts newline.
        if (e.metaKey || e.ctrlKey) {
          e.preventDefault();
          void submit();
        }
      } else {
        // Enter mode: Enter sends (unless Shift is pressed); Cmd/Ctrl+Enter also sends.
        if (e.metaKey || e.ctrlKey || !e.shiftKey) {
          e.preventDefault();
          void submit();
        }
      }
    }
  };

  return {
    draft,
    setDraft,
    onDraftChange,
    onCompositionStart,
    onCompositionEnd,
    attachments,
    attachError,
    attaching,
    addImageFiles,
    onAttach,
    removeAttachment,
    temporaryMode,
    setTemporaryMode,
    allowTools,
    setAllowTools,
    isDragging,
    setIsDragging,
    textareaRef,
    mentionQuery,
    mentionIndex,
    setMentionQuery,
    setMentionIndex,
    slashQuery,
    slashIndex,
    setSlashQuery,
    setSlashIndex,
    checkMention,
    checkSlash,
    onSelectMention,
    onSelectSlashCommand,
    onInsertMention,
    handleDropFiles,
    onSubmit: submit,
    onKeyDown,
    hasImage,
    visionWarning,
    sendDisabledReason,
    canSend,
    listening,
    toggleVoiceInput,
    speechSupported,
  };
}
