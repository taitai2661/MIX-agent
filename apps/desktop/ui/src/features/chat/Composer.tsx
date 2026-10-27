import { memo, useCallback, useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  ArrowUp,
  AtSign,
  Bot,
  Brain,
  Check,
  EyeOff,
  MessageSquare,
  Mic,
  Paperclip,
  Square,
  Wrench,
  X,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { chord } from "@/lib/platform";
import { getString, removeKey, setString } from "@/lib/storage";
import { subscribePromptInsert } from "@/lib/promptInsert";
import { loadPromptHistory, pushPromptHistory } from "@/lib/promptHistory";
import { useUi, type SendShortcut } from "@/lib/ui";
import type { ModelView, ProjectView, ProviderView } from "@/lib/types";
import { Alert } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
  DropdownMenuTriggerButton,
} from "@/components/ui/dropdown-menu";
import { Kbd } from "@/components/layout";
import { Spinner } from "@/components/ui/spinner";
import { ErrorBox } from "@/components/shared";
import { ModelPicker } from "./ModelPicker";
import { ProjectMenu } from "./ProjectMenu";
import { MentionPopover } from "./MentionPopover";
import { SlashCommandPopover, type SlashCommand } from "./SlashCommandPopover";
import { MAX_ATTACHMENTS, useAttachments } from "./useAttachments";
import { useVoiceInput } from "./useVoiceInput";
import {
  SHARED_EMPTY_MODELS,
  SHARED_EMPTY_PROJECTS,
  SHARED_EMPTY_PROVIDERS,
  useChatComposerContext,
} from "./ChatComposerContext";

export interface DraftAttachment {
  kind: string;
  mime_type: string;
  data_base64: string;
  filename: string;
}

interface ModeMeta {
  id: "chat" | "thinking" | "agent";
  label: string;
  hint: string;
  Icon: typeof MessageSquare;
}

const ALL_MODES: ModeMeta[] = [
  // Labels stay as raw i18n keys; `t()` is applied at render time so a
  // locale switch updates them without re-importing the module.
  {
    id: "chat",
    label: "チャット",
    hint: "素早い質疑応答",
    Icon: MessageSquare,
  },
  { id: "thinking", label: "思考", hint: "深く考えて回答", Icon: Brain },
  {
    id: "agent",
    label: "エージェント",
    hint: "ツールで自律的に作業",
    Icon: Bot,
  },
];

/** Stable no-op for the provider-less fallback path (tests pass real ones). */
const NOOP_SET_MODE: (mode: "chat" | "thinking" | "agent") => void = () => {};
const NOOP_SET_MODEL: (id: string) => void = () => {};

export const Composer = memo(function Composer(props: {
  /** Direct-from-store defaults apply when omitted: ChatPane renders
   *  `<Composer onSend onStop busy draftKey />` inside a
   *  `ChatComposerProvider` and no longer drills these. Explicit props still
   *  win (tests, previews). */
  mode?: "chat" | "thinking" | "agent";
  setMode?: (mode: "chat" | "thinking" | "agent") => void;
  /** Modes the active surface exposes. When empty, fall back to ALL_MODES. */
  allowedModes?: ReadonlyArray<"chat" | "thinking" | "agent">;
  models?: ModelView[];
  modelId?: string;
  setModelId?: (id: string) => void;
  /** Providers, for resolving a model's provider name in the picker. */
  providers?: ProviderView[];
  projects?: ProjectView[];
  projectId?: string | null;
  onProjectChange?: (id: string | null) => void;
  onCreateProject?: (name: string) => Promise<string | null>;
  /** While true the model/project lists are "not known yet", not "empty". The
   *  menus used to say "No model configured — add a provider first" on every
   *  open that happened before the query settled. */
  modelsPending?: boolean;
  projectsPending?: boolean;
  busy: boolean;
  onSend: (
    content: string,
    attachments: DraftAttachment[],
    options: { temporaryMode: boolean; allowTools: boolean },
  ) => void;
  onStop: () => void;
  /** Key shortcut used to send messages (Enter vs Cmd/Ctrl+Enter). */
  sendShortcut?: SendShortcut;
  /** Drafts are scoped per conversation so switching threads no longer drops
   *  (or leaks) what the user was typing. `null` is the "no thread yet" slot. */
  draftKey?: string | null;
}) {
  // UI state comes from the provider when the caller omits it. `useUi`
  // selectors inside the provider keep this subscribed per-field, so run
  // streaming (~16 store flushes/s) does not re-render the composer.
  const ctx = useChatComposerContext();
  const storeSendShortcut = useUi((s) => s.sendShortcut);
  const sendShortcut = props.sendShortcut ?? storeSendShortcut;
  const mode = props.mode ?? ctx?.mode ?? "chat";
  const setMode = props.setMode ?? ctx?.setMode ?? NOOP_SET_MODE;
  const allowedModes = props.allowedModes ?? ctx?.allowedModes;
  const models = props.models ?? ctx?.models ?? SHARED_EMPTY_MODELS;
  const modelId = props.modelId ?? ctx?.modelId ?? "auto";
  const setModelId = props.setModelId ?? ctx?.setModelId ?? NOOP_SET_MODEL;
  const providers = props.providers ?? ctx?.providers ?? SHARED_EMPTY_PROVIDERS;
  const projects = props.projects ?? ctx?.projects ?? SHARED_EMPTY_PROJECTS;
  const projectId = props.projectId ?? ctx?.projectId ?? null;
  const onProjectChange = props.onProjectChange ?? ctx?.onProjectChange;
  const onCreateProject = props.onCreateProject ?? ctx?.onCreateProject;
  const modelsPending = props.modelsPending ?? ctx?.modelsPending ?? false;
  const projectsPending =
    props.projectsPending ?? ctx?.projectsPending ?? false;
  const { busy, onSend, onStop, draftKey = null } = props;
  const [draft, setDraft] = useState("");
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
  const draftKeyRef = useRef<string | null>(draftKey ?? null);
  const draftRef = useRef(draft);
  draftRef.current = draft;

  const historyIndexRef = useRef<number>(-1);
  const historyDraftRef = useRef<string>("");

  const activeProject = projects.find((p) => p.id === projectId);
  const rootPath = activeProject?.rootPath ?? null;

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

  // Draft persistence: one slot per conversation, via `lib/storage.ts`
  // (best-effort). Switching threads used to either drop the draft or leak it
  // into the next thread. The ref dance avoids persisting the freshly-restored
  // text of the new thread back into the old thread's slot.
  useEffect(() => {
    const key = draftKey ?? null;
    if (key !== draftKeyRef.current) {
      const prev = draftKeyRef.current;
      const current = draftRef.current;
      // Persist the old thread's text outside the state updater (updaters
      // must be pure; side-effects here double-fire under StrictMode).
      if (prev !== undefined) {
        if (current) setString(`mix:composer-draft:${prev ?? "none"}`, current);
        else removeKey(`mix:composer-draft:${prev ?? "none"}`);
      }
      draftKeyRef.current = key;
      setDraft(getString(`mix:composer-draft:${key ?? "none"}`) ?? "");
    }
  }, [draftKey]);

  useEffect(() => {
    const key = draftKeyRef.current ?? "none";
    // Keystrokes fire per character; debounce the write so typing doesn't
    // hit storage (a synchronous disk read) on every frame.
    const timer = window.setTimeout(() => {
      if (draft) setString(`mix:composer-draft:${key}`, draft);
      else removeKey(`mix:composer-draft:${key}`);
    }, 300);
    return () => window.clearTimeout(timer);
  }, [draft]);

  // Focus the box on first mount so the user can type immediately.
  useEffect(() => {
    const id = window.requestAnimationFrame(() => textareaRef.current?.focus());
    return () => window.cancelAnimationFrame(id);
  }, []);

  const selectedModel =
    modelId === "auto" ? undefined : models.find((m) => m.id === modelId);
  const isAutoModel = modelId === "auto";
  // The selected model is gone from the list — its provider was deleted, or the
  // models failed to load while an id was already set. Sending would queue a run
  // against a model id no provider can serve, and the failure only surfaces
  // after the run starts, so it is blocked here instead. The `models.length > 0`
  // guard keeps an empty list from reading as "stale" on first load, which would
  // hide the real state (nothing is configured yet). `auto` is a pseudo-model
  // resolved server-side and is never stale.
  const staleModel =
    !!modelId && !isAutoModel && models.length > 0 && !selectedModel;
  // Narrow the master mode list down to what the active surface allows. When
  // the surface only allows one mode we hide the dropdown entirely below.
  const MODES =
    allowedModes && allowedModes.length > 0
      ? ALL_MODES.filter((m) => allowedModes.includes(m.id))
      : ALL_MODES;
  const activeMode = MODES.find((m) => m.id === mode) ?? MODES[0];
  // Project context only matters for the agent surface; chat surface hides
  // the picker to keep the toolbar minimal.
  const showProjectPicker = MODES.some((m) => m.id === "agent");
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
      mode?: typeof mode;
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

  const submit = () => {
    if (!canSend) return;
    const trimmed = draft.trim();
    pushPromptHistory(trimmed);
    historyIndexRef.current = -1;
    historyDraftRef.current = "";
    onSend(trimmed, attachments, { temporaryMode, allowTools });
    setDraft("");
    clearAttachments();
    removeKey(`mix:composer-draft:${draftKeyRef.current ?? "none"}`);
    // Temporary mode is per-send: leaving it on would silently discard the next
    // turn's transcript too.
    setTemporaryMode(false);
    setAllowTools(false);
    textareaRef.current?.focus();
  };

  const placeholder =
    mode === "agent"
      ? t("エージェントにタスクを指示… (ターミナル実行・ファイル編集が可能)")
      : mode === "thinking"
        ? t("深く考えてほしいことを入力…")
        : t("MIX-agent に話しかける…");

  return (
    // Fixed toolbar: every control is always rendered with truncation, so the
    // row never gains or loses buttons while the window or side panels move.
    // (Container queries used to hide the mode label / project picker below
    // 18–24rem, which flickered the whole row during every panel drag.)
    <div className="border-t border-line bg-bg px-4 pb-4 pt-3">
      <div className="mx-auto flex w-full max-w-content flex-col gap-2">
        <div
          onDragOver={(e) => {
            if ([...(e.dataTransfer?.types ?? [])].includes("Files")) {
              e.preventDefault();
              setIsDragging(true);
            }
          }}
          onDragLeave={(e) => {
            if (e.currentTarget.contains(e.relatedTarget as Node)) return;
            setIsDragging(false);
          }}
          onDrop={(e) => {
            e.preventDefault();
            setIsDragging(false);
            const files = [...(e.dataTransfer?.files ?? [])];
            if (files.length > 0) {
              void handleDropFiles(files);
            }
          }}
          className={cn(
            "relative flex flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-sm",
            "transition-[border-color,box-shadow] duration-200 ease-out",
            "hover:border-line-strong",
            "focus-within:border-brand/60 focus-within:shadow-md focus-within:ring-4 focus-within:ring-brand/10",
            isDragging && "border-brand ring-4 ring-brand/20 bg-brand-soft/20",
          )}
        >
          {isDragging && (
            <div className="pointer-events-none absolute inset-0 z-30 flex flex-col items-center justify-center rounded-2xl border-2 border-dashed border-brand bg-surface/90 backdrop-blur-xs">
              <Paperclip size={22} className="text-brand animate-bounce" />
              <span className="mt-1 text-xs font-semibold text-brand">
                {t("ファイルをドロップして添付")}
              </span>
            </div>
          )}
          {attachments.length > 0 && (
            <div className="flex flex-wrap gap-1.5 border-b border-line px-3 py-2">
              {attachments.map((a, i) => (
                <span
                  key={`${a.filename}-${i}`}
                  className={cn(
                    "inline-flex items-center gap-1 rounded-md border border-line bg-inset px-1.5 py-0.5 text-xs text-muted",
                    "transition-colors hover:border-line-strong",
                  )}
                >
                  {a.mime_type.startsWith("image/") ? (
                    <img
                      src={`data:${a.mime_type};base64,${a.data_base64}`}
                      alt={a.filename}
                      className="size-5 rounded-sm object-cover"
                    />
                  ) : (
                    <Paperclip size={12} className="shrink-0" />
                  )}
                  <span className="max-w-40 truncate">{a.filename}</span>
                  {a.mime_type.startsWith("image/") && visionWarning && (
                    <AlertTriangle
                      size={12}
                      role="img"
                      aria-label={t("このモデルは画像入力に対応していません")}
                      className="text-warning"
                    />
                  )}
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => removeAttachment(i)}
                    aria-label={`${a.filename} ${t("削除")}`}
                  >
                    <X size={13} />
                  </Button>
                </span>
              ))}
              <span className="self-center text-xs text-faint">
                {attachments.length} / {MAX_ATTACHMENTS}
              </span>
            </div>
          )}

          <div className="relative">
            {mentionQuery !== null && rootPath && (
              <MentionPopover
                rootPath={rootPath}
                query={mentionQuery}
                onSelect={onSelectMention}
                onClose={() => setMentionQuery(null)}
                selectedIndex={mentionIndex}
                setSelectedIndex={setMentionIndex}
              />
            )}
            {slashQuery !== null && mentionQuery === null && (
              <SlashCommandPopover
                query={slashQuery}
                onSelect={onSelectSlashCommand}
                selectedIndex={slashIndex}
                setSelectedIndex={setSlashIndex}
              />
            )}
            <textarea
              ref={textareaRef}
              rows={1}
              aria-label={t("メッセージを入力")}
              aria-describedby="composer-hints"
              placeholder={placeholder}
              value={draft}
              onChange={(e) => {
                setDraft(e.target.value);
                historyIndexRef.current = -1;
              }}
              onCompositionStart={() => {
                isComposingRef.current = true;
              }}
              onCompositionEnd={() => {
                isComposingRef.current = false;
                compositionEndTimestampRef.current = Date.now();
              }}
              onKeyUp={(e) => {
                if (
                  !["ArrowUp", "ArrowDown", "Enter", "Tab", "Escape"].includes(
                    e.key,
                  )
                ) {
                  checkMention();
                  checkSlash();
                }
              }}
              onClick={() => {
                checkMention();
                checkSlash();
              }}
              onPaste={(e) => {
                const files = [...(e.clipboardData?.files ?? [])].filter((f) =>
                  f.type.startsWith("image/"),
                );
                if (files.length > 0) {
                  e.preventDefault();
                  void addImageFiles(files);
                }
              }}
              onDrop={(e) => {
                e.preventDefault();
                setIsDragging(false);
                const files = [...(e.dataTransfer?.files ?? [])];
                if (files.length > 0) {
                  void handleDropFiles(files);
                }
              }}
              onDragOver={(e) => {
                if ([...(e.dataTransfer?.types ?? [])].includes("Files")) {
                  e.preventDefault();
                  setIsDragging(true);
                }
              }}
              onKeyDown={(e) => {
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
                    const items = document.querySelectorAll(
                      "[data-mention-item]",
                    );
                    const active = items[mentionIndex] as
                      | HTMLElement
                      | undefined;
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
                    const items =
                      document.querySelectorAll("[data-slash-item]");
                    const active = items[slashIndex] as HTMLElement | undefined;
                    if (active) {
                      e.preventDefault();
                      active.click();
                      return;
                    }
                  }
                }

                if (
                  mentionQuery === null &&
                  slashQuery === null &&
                  !isImeComposing
                ) {
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
                  } else if (
                    e.key === "ArrowDown" &&
                    historyIndexRef.current >= 0
                  ) {
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
                      submit();
                    }
                  } else {
                    // Enter mode: Enter sends (unless Shift is pressed); Cmd/Ctrl+Enter also sends.
                    if (e.metaKey || e.ctrlKey || !e.shiftKey) {
                      e.preventDefault();
                      submit();
                    }
                  }
                }
              }}
              className="max-h-56 min-h-16 w-full resize-none bg-transparent px-5 pt-4 pb-3 text-base leading-6 text-fg placeholder:text-faint outline-none"
            />
          </div>

          {/* Toolbar: two groups (input controls / model+actions) that wrap as a
              row on narrow centres instead of pushing the send button out of
              view. The right group owns `ms-auto`, not a bare spacer, so wrapping
              keeps the mic/send together at the row end. */}
          <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-2 border-t border-line px-3 pt-2 pb-2.5">
            <Button
              variant="ghost"
              size="icon"
              onClick={() => void onAttach()}
              disabled={attaching || attachments.length >= MAX_ATTACHMENTS}
              title={`${t("画像を添付")} — ${t("貼り付け・ドロップにも対応")}`}
              aria-label={t("画像を添付")}
              className="shrink-0 rounded-full"
            >
              {attaching ? <Spinner size={18} /> : <Paperclip size={18} />}
            </Button>

            <Button
              variant="ghost"
              size="icon"
              onClick={() => {
                const el = textareaRef.current;
                if (!el) return;
                const pos = el.selectionStart;
                const textBefore = el.value.slice(0, pos);
                const textAfter = el.value.slice(pos);
                const prefix =
                  textBefore.length > 0 && !textBefore.endsWith(" ")
                    ? " @"
                    : "@";
                setDraft(textBefore + prefix + textAfter);
                setMentionQuery("");
                requestAnimationFrame(() => {
                  el.focus();
                  const nextPos = pos + prefix.length;
                  el.setSelectionRange(nextPos, nextPos);
                });
              }}
              title={t("ファイルをメンション (@)")}
              aria-label={t("ファイルをメンション (@)")}
              className="shrink-0 rounded-full"
            >
              <AtSign size={18} />
            </Button>

            {MODES.length > 1 ? (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <DropdownMenuTriggerButton
                    aria-label={t("モード")}
                    title={t(activeMode.label)}
                    className="max-w-28 shrink rounded-full border-transparent bg-transparent hover:bg-hover sm:max-w-56"
                  >
                    <activeMode.Icon
                      size={13}
                      className="shrink-0 text-muted"
                      aria-hidden="true"
                    />
                    {/* Always visible with truncation: hiding the label at narrow
                        widths used to reflow the whole toolbar mid-resize. */}
                    <span className="min-w-0 max-w-20 truncate">
                      {t(activeMode.label)}
                    </span>
                  </DropdownMenuTriggerButton>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start">
                  <DropdownMenuLabel>{t("モード")}</DropdownMenuLabel>
                  {MODES.map((m) => (
                    <DropdownMenuItem
                      key={m.id}
                      onSelect={() => setMode(m.id)}
                      description={t(m.hint)}
                    >
                      <m.Icon
                        size={13}
                        className="shrink-0 text-muted"
                        aria-hidden="true"
                      />
                      <span className="min-w-0 flex-1 truncate font-medium">
                        {t(m.label)}
                      </span>
                      {m.id === mode && (
                        <Check
                          size={13}
                          className="ms-auto shrink-0 text-brand"
                          aria-hidden="true"
                        />
                      )}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            ) : (
              // Single-mode surface (e.g. desktop-agent → "agent"): render a
              // static badge so the toolbar still announces what mode is active.
              <span
                title={t(activeMode.label)}
                className="inline-flex h-control min-w-0 shrink items-center gap-1.5 rounded-full px-2.5 text-xs font-medium text-fg"
              >
                <activeMode.Icon
                  size={13}
                  className="shrink-0 text-muted"
                  aria-hidden="true"
                />
                <span className="min-w-0 max-w-20 truncate">
                  {t(activeMode.label)}
                </span>
              </span>
            )}

            <div className="ms-auto flex min-w-0 flex-wrap items-center justify-end gap-x-1.5 gap-y-2">
              <ModelPicker
                models={models}
                value={modelId}
                onChange={setModelId}
                providers={providers}
                pending={modelsPending}
                temporary={temporaryMode}
                onTemporaryChange={setTemporaryMode}
                allowTools={allowTools}
                onAllowToolsChange={setAllowTools}
                className="max-w-32 min-w-0 flex-1 rounded-full sm:max-w-44 sm:flex-none"
              />

              {showProjectPicker && (
                <ProjectMenu
                  projects={projects}
                  projectId={projectId}
                  onProjectChange={onProjectChange}
                  onCreateProject={onCreateProject}
                  pending={projectsPending}
                />
              )}

              {/* Thin divider groups the text controls (left) from the voice +
                send actions (right), so the model name, mic and send button no
                longer read as one crowded run. */}
              <span
                aria-hidden="true"
                className="mx-1 h-5 w-px shrink-0 bg-line"
              />

              <Button
                variant="ghost"
                size="icon"
                onClick={toggleVoiceInput}
                disabled={!speechSupported}
                title={
                  speechSupported
                    ? t("音声入力")
                    : `${t("音声入力")} — ${t("このブラウザは音声入力に対応していません")}`
                }
                aria-label={t("音声入力")}
                aria-disabled={!speechSupported}
                className={cn("shrink-0 rounded-full", listening && "relative")}
              >
                <Mic size={18} />
                {listening && (
                  <span
                    aria-hidden="true"
                    className="pointer-events-none absolute inset-0 rounded-full border border-current opacity-60 animate-mix-pulse motion-reduce:animate-none"
                  />
                )}
              </Button>

              {/* Stop and send share one `size="icon"` slot, so both glyphs are
                the same 18px and swapping between them does not shift the row. */}
              {busy ? (
                <Button
                  variant="default"
                  size="icon"
                  onClick={onStop}
                  aria-label={t("停止")}
                  title={t("停止")}
                  className="shrink-0 rounded-full shadow-md"
                >
                  <Square size={18} />
                </Button>
              ) : (
                <Button
                  variant="primary"
                  size="icon"
                  disabled={!canSend}
                  onClick={submit}
                  aria-label={
                    sendDisabledReason
                      ? `${t("送信")} — ${sendDisabledReason}`
                      : t("送信")
                  }
                  aria-keyshortcuts={
                    sendShortcut === "modifier"
                      ? "meta+Enter ctrl+Enter"
                      : "Enter meta+Enter ctrl+Enter"
                  }
                  title={
                    sendDisabledReason ??
                    `${t("送信")} (${sendShortcut === "modifier" ? chord("Mod", "Enter") : "Enter"})`
                  }
                  className="shrink-0 rounded-full shadow-md disabled:shadow-none"
                >
                  <ArrowUp size={18} />
                </Button>
              )}
            </div>
          </div>
        </div>

        <div
          id="composer-hints"
          className="flex min-h-5 flex-wrap items-center gap-x-3 gap-y-1 px-1 text-xs text-faint"
        >
          {sendShortcut === "modifier" ? (
            <span
              className="hidden items-center gap-1 sm:inline-flex"
              aria-hidden="true"
            >
              <Kbd>{chord("Mod", "Enter")}</Kbd>
              <span>{t("送信")}</span>
              <span className="mx-1 text-line">·</span>
              <Kbd>Enter</Kbd>
              <span>{t("改行")}</span>
            </span>
          ) : (
            <span
              className="hidden items-center gap-1 sm:inline-flex"
              aria-hidden="true"
            >
              <Kbd>Enter</Kbd>
              <span>{t("送信")}</span>
              <span className="mx-1 text-line">·</span>
              <Kbd>{chord("Shift", "Enter")}</Kbd>
              <span>{t("改行")}</span>
            </span>
          )}
          <span className="min-w-0 flex-1" />
          {temporaryMode && (
            <Badge tone="warning">
              <EyeOff size={10} aria-hidden="true" />
              {t("一時モード: この送信は保存されません")}
            </Badge>
          )}
          {/* Always in the tree with a reserved width: mounting the counter on
              the first keystroke used to shift the whole hint row. */}
          <span
            className={cn(
              "min-w-16 text-end font-mono tabular-nums",
              draft.length === 0 && "invisible",
            )}
            aria-live="off"
            aria-hidden={draft.length === 0}
          >
            {draft.length.toLocaleString()} {t("文字")}
          </span>
          {!modelId && models.length > 0 && (
            <Badge tone="warning">
              <Wrench size={10} aria-hidden="true" />
              {t("モデル未設定")}
            </Badge>
          )}
          {staleModel && (
            <Badge tone="danger">
              <Wrench size={10} aria-hidden="true" />
              {t("選択したモデルが利用できません")}
            </Badge>
          )}
        </div>

        {staleModel && (
          <ErrorBox
            error={t(
              "選択したモデルのプロバイダーが削除されています。モデルを再選択してください。",
            )}
          />
        )}

        {visionWarning && (
          <Alert tone="warning">
            {t("画像を添付するにはVision対応モデルを選択してください")}
          </Alert>
        )}
        {attachError && <ErrorBox error={attachError} />}
      </div>
    </div>
  );
});
