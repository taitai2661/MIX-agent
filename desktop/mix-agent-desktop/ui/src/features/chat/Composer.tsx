import { memo, useEffect, useRef, useState } from "react";
import {
  AlertTriangle, ArrowUp, Bot, Brain, Check, ChevronDown, ChevronLeft, Eye, Folder, Loader2,
  MessageSquare, Mic, Paperclip, Plus, Square, Wrench, X,
} from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import type { ModelView, ProjectView } from "@/lib/types";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem,
  DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";

export interface DraftAttachment {
  kind: string;
  mime_type: string;
  data_base64: string;
  filename: string;
}

/** Stable default, so the memo below is not defeated by a fresh `[]` on every
 * render of the parent. */
const EMPTY_PROJECTS: ProjectView[] = [];

interface ModeMeta {
  id: "chat" | "thinking" | "agent";
  label: string;
  hint: string;
  Icon: typeof MessageSquare;
}

const ALL_MODES: ModeMeta[] = [
  { id: "chat", label: "チャット", hint: "素早い質疑応答", Icon: MessageSquare },
  { id: "thinking", label: "思考", hint: "深く考えて回答", Icon: Brain },
  { id: "agent", label: "エージェント", hint: "ツールで自律的に作業", Icon: Bot },
];

const MAX_ATTACHMENTS = 8;

export const Composer = memo(function Composer({
  mode,
  setMode,
  allowedModes,
  models,
  modelId,
  setModelId,
  projects = EMPTY_PROJECTS,
  projectId = null,
  onProjectChange,
  onCreateProject,
  busy,
  onSend,
  onStop,
}: {
  mode: "chat" | "thinking" | "agent";
  setMode: (mode: "chat" | "thinking" | "agent") => void;
  /** Modes the active surface exposes. When empty, fall back to ALL_MODES. */
  allowedModes?: ReadonlyArray<"chat" | "thinking" | "agent">;
  models: ModelView[];
  modelId: string;
  setModelId: (id: string) => void;
  projects?: ProjectView[];
  projectId?: string | null;
  onProjectChange?: (id: string | null) => void;
  onCreateProject?: (name: string) => Promise<string | null>;
  busy: boolean;
  onSend: (content: string, attachments: DraftAttachment[]) => void;
  onStop: () => void;
}) {
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState<DraftAttachment[]>([]);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [attaching, setAttaching] = useState(false);
  const [listening, setListening] = useState(false);
  const [projectMenuOpen, setProjectMenuOpen] = useState(false);
  const [projectMenuMode, setProjectMenuMode] = useState<"list" | "create">("list");
  const [projectDraftName, setProjectDraftName] = useState("");
  const [projectCreating, setProjectCreating] = useState(false);
  const [projectCreateError, setProjectCreateError] = useState<string | null>(null);
  const projectNameRef = useRef<HTMLInputElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const speechRef = useRef<{ stop: () => void; abort: () => void } | null>(null);

  const selectedModel = models.find((m) => m.id === modelId);
  // Narrow the master mode list down to what the active surface allows. When
  // the surface only allows one mode we hide the dropdown entirely below.
  const MODES = allowedModes && allowedModes.length > 0
    ? ALL_MODES.filter((m) => allowedModes.includes(m.id))
    : ALL_MODES;
  const activeMode = MODES.find((m) => m.id === mode) ?? MODES[0];
  const activeProject = projects.find((p) => p.id === projectId) ?? null;
  // Project context only matters for the agent surface; chat surface hides
  // the picker to keep the toolbar minimal.
  const showProjectPicker = MODES.some((m) => m.id === "agent");
  const hasImage = attachments.some((a) => a.mime_type.startsWith("image/"));
  const visionWarning = hasImage && selectedModel != null && !selectedModel.supports_vision;
  const canSend =
    draft.trim().length > 0
    && !busy
    && !!modelId
    && (!hasImage || !selectedModel || selectedModel.supports_vision);

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [draft]);

  useEffect(() => () => speechRef.current?.abort(), []);

  // Welcome chips (or external callers) can pre-fill the draft via this event
  // instead of having to thread state through ChatPane.
  useEffect(() => {
    const onInsert = (event: Event) => {
      const detail = (event as CustomEvent<{ text?: string; mode?: typeof mode }>).detail;
      if (typeof detail?.text === "string") setDraft(detail.text);
      if (detail?.mode && (detail.mode === "chat" || detail.mode === "thinking" || detail.mode === "agent")) {
        setMode(detail.mode);
      }
      window.requestAnimationFrame(() => textareaRef.current?.focus());
    };
    window.addEventListener("mix:insert-prompt", onInsert);
    return () => window.removeEventListener("mix:insert-prompt", onInsert);
  }, [setMode]);

  const toggleVoiceInput = () => {
    if (speechRef.current) {
      speechRef.current.stop();
      speechRef.current = null;
      setListening(false);
      return;
    }

    type Recognition = {
      lang: string;
      interimResults: boolean;
      onresult: ((event: { resultIndex: number; results: ArrayLike<ArrayLike<{ transcript: string; isFinal: boolean }>> }) => void) | null;
      onend: (() => void) | null;
      onerror: (() => void) | null;
      start: () => void;
      stop: () => void;
      abort: () => void;
    };
    type RecognitionWindow = Window & {
      SpeechRecognition?: new () => Recognition;
      webkitSpeechRecognition?: new () => Recognition;
    };
    const RecognitionCtor = (window as RecognitionWindow).SpeechRecognition
      ?? (window as RecognitionWindow).webkitSpeechRecognition;
    if (!RecognitionCtor) return;

    const recognition = new RecognitionCtor();
    const initialDraft = draft.trimEnd();
    let spoken = "";
    recognition.lang = "ja-JP";
    recognition.interimResults = true;
    recognition.onresult = (event) => {
      let interim = "";
      for (let i = event.resultIndex; i < event.results.length; i += 1) {
        const result = event.results[i];
        if (result[0].isFinal) spoken += result[0].transcript;
        else interim += result[0].transcript;
      }
      setDraft([initialDraft, spoken, interim].filter(Boolean).join(initialDraft ? " " : ""));
    };
    recognition.onend = () => {
      speechRef.current = null;
      setListening(false);
    };
    recognition.onerror = () => {
      speechRef.current = null;
      setListening(false);
    };
    speechRef.current = recognition;
    setListening(true);
    recognition.start();
  };

  const submit = () => {
    if (!canSend) return;
    onSend(draft.trim(), attachments);
    setDraft("");
    setAttachments([]);
    textareaRef.current?.focus();
  };

  const onAttach = async () => {
    setAttachError(null);
    if (attachments.length >= MAX_ATTACHMENTS) return;
    setAttaching(true);
    try {
      const picked = await openDialog({
        multiple: true,
        filters: [
          {
            name: "images",
            extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp"],
          },
        ],
      });
      const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
      const remaining = MAX_ATTACHMENTS - attachments.length;
      for (const path of paths.slice(0, remaining)) {
        try {
          const payload = await api.readAttachment(path);
          setAttachments((prev) => [
            ...prev,
            {
              kind: payload.kind,
              mime_type: payload.mime_type,
              data_base64: payload.data_base64,
              filename: payload.filename,
            },
          ]);
        } catch (e) {
          setAttachError(String(e));
        }
      }
    } finally {
      setAttaching(false);
    }
  };

  const submitNewProject = async () => {
    const name = projectDraftName.trim();
    if (!name || projectCreating || !onCreateProject) return;
    setProjectCreating(true);
    setProjectCreateError(null);
    try {
      const created = await onCreateProject(name);
      // `null` means the user backed out (cancelled the folder picker):
      // keep the form open so the draft name is not thrown away.
      if (!created) return;
      setProjectDraftName("");
      setProjectCreateError(null);
      setProjectMenuMode("list");
      setProjectMenuOpen(false);
    } catch (e) {
      setProjectCreateError(String(e));
    } finally {
      setProjectCreating(false);
    }
  };

  const cancelNewProject = () => {
    setProjectDraftName("");
    setProjectCreateError(null);
    setProjectMenuMode("list");
  };

  const resetProjectMenu = (open: boolean) => {
    setProjectMenuOpen(open);
    if (!open) {
      setProjectMenuMode("list");
      setProjectDraftName("");
      setProjectCreateError(null);
    }
  };

  useEffect(() => {
    if (projectMenuMode === "create") {
      const id = window.setTimeout(() => projectNameRef.current?.focus(), 0);
      return () => window.clearTimeout(id);
    }
    return undefined;
  }, [projectMenuMode]);

  const placeholder = mode === "agent"
    ? t("複雑な作業を依頼…")
    : mode === "thinking"
      ? t("深く考えてほしいことを入力…")
      : t("MIX-agent に話しかける…");

  return (
    <div className="border-t border-line bg-bg px-6 pb-4 pt-3">
      <div className="mx-auto flex w-full max-w-[820px] flex-col gap-2">
        <div
          className={cn(
            "flex flex-col rounded-xl border border-line bg-surface shadow-xs",
            "transition-colors duration-150 ease-out",
            "focus-within:border-brand focus-within:shadow-sm",
          )}
        >
          {attachments.length > 0 && (
            <div className="flex flex-wrap gap-1.5 border-b border-line px-3 py-2">
              {attachments.map((a, i) => (
                <span
                  key={`${a.filename}-${i}`}
                  className={cn(
                    "inline-flex items-center gap-1.5 rounded-md border border-line bg-inset py-0.5 pl-1.5 pr-1 text-xs text-muted",
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
                    <Paperclip size={12} className="shrink-0 text-faint" />
                  )}
                  <span className="max-w-40 truncate">{a.filename}</span>
                  {a.mime_type.startsWith("image/") && visionWarning && (
                    <span
                      className="inline-flex items-center text-warning"
                      title={t("このモデルは画像入力に対応していません")}
                    >
                      <AlertTriangle size={12} aria-label={t("このモデルは画像入力に対応していません")} />
                    </span>
                  )}
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => setAttachments((prev) => prev.filter((_, j) => j !== i))}
                    aria-label={`${a.filename} ${t("削除")}`}
                  >
                    <X size={11} />
                  </Button>
                </span>
              ))}
              <span className="inline-flex items-center gap-1 self-center text-2xs text-faint">
                {attachments.length} / {MAX_ATTACHMENTS}
              </span>
            </div>
          )}

          <textarea
            ref={textareaRef}
            rows={1}
            aria-label={t("メッセージを入力")}
            placeholder={placeholder}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                submit();
              }
            }}
            className={cn(
              "max-h-56 min-h-16 w-full resize-none bg-transparent px-3 py-2.5",
              "text-sm leading-6 text-fg outline-none placeholder:text-faint",
            )}
          />

          <div className="flex items-center gap-1 px-2 pb-2">
            <Button
              variant="ghost"
              size="icon"
              onClick={() => void onAttach()}
              disabled={attaching || attachments.length >= MAX_ATTACHMENTS}
              title={t("画像を添付")}
              aria-label={t("画像を添付")}
            >
              {attaching
                ? <Loader2 size={17} className="spin" />
                : <Paperclip size={17} strokeWidth={1.8} />}
            </Button>

            {MODES.length > 1 ? (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button
                    type="button"
                    aria-label={t("モード")}
                    title={t(activeMode.label)}
                    className={cn(
                      "inline-flex h-8 items-center gap-1.5 rounded-md border border-line bg-surface px-2.5",
                      "text-xs font-medium text-fg",
                      "transition-colors duration-100 hover:bg-hover",
                      "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-focus",
                      "data-[state=open]:border-line-strong",
                    )}
                  >
                    <activeMode.Icon size={13} className="shrink-0 text-muted" aria-hidden="true" />
                    <span>{t(activeMode.label)}</span>
                    <ChevronDown size={11} className="text-muted" aria-hidden="true" />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start">
                  <DropdownMenuLabel>{t("モード")}</DropdownMenuLabel>
                  {MODES.map((m) => (
                    <DropdownMenuItem
                      key={m.id}
                      onSelect={() => setMode(m.id)}
                      className="flex-col items-start gap-0 py-1.5"
                    >
                      <span className="flex w-full items-center gap-2">
                        <m.Icon size={13} className="shrink-0 text-muted" aria-hidden="true" />
                        <span className="font-medium">{t(m.label)}</span>
                        {m.id === mode && (
                          <Check size={13} className="ml-auto text-brand" aria-hidden="true" />
                        )}
                      </span>
                      <span className="ps-5 text-2xs text-faint">{t(m.hint)}</span>
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            ) : (
              // Single-mode surface (e.g. Codex → "agent"): render a static
              // badge so the toolbar still announces what mode is active.
              <span
                aria-label={t("モード")}
                title={t(activeMode.label)}
                className="inline-flex h-8 items-center gap-1.5 rounded-md border border-line bg-surface px-2.5 text-xs font-medium text-fg"
              >
                <activeMode.Icon size={13} className="shrink-0 text-muted" aria-hidden="true" />
                <span>{t(activeMode.label)}</span>
              </span>
            )}

            <span className="flex-1" />

            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <button
                  type="button"
                  aria-label={t("モデル")}
                  title={selectedModel?.label ?? (models.length === 0 ? t("モデル未設定") : "")}
                  className={cn(
                    "inline-flex h-8 max-w-56 items-center gap-1.5 rounded-md border border-line bg-surface px-2.5",
                    "text-xs font-medium text-fg",
                    "transition-colors duration-100 hover:bg-hover",
                    "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-focus",
                    "data-[state=open]:border-line-strong",
                  )}
                >
                  <span className="min-w-0 flex-1 truncate text-start">
                    {selectedModel?.label ?? (models.length === 0 ? t("モデル未設定") : t("モデル"))}
                  </span>
                  {selectedModel?.supports_vision && (
                    <Eye size={11} className="shrink-0 text-faint" aria-label="vision" />
                  )}
                  <ChevronDown size={11} className="shrink-0 text-muted" aria-hidden="true" />
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="min-w-64">
                <DropdownMenuLabel>{t("モデル")}</DropdownMenuLabel>
                {models.length === 0 && (
                  <div className="px-2 py-1.5 text-2xs text-faint">
                    {t("モデルが未設定です。先にプロバイダーを追加してください。")}
                  </div>
                )}
                {models.map((m) => (
                  <DropdownMenuItem
                    key={m.id}
                    onSelect={() => setModelId(m.id)}
                    className="flex-col items-start gap-0 py-1.5"
                  >
                    <span className="flex w-full items-center gap-2">
                      <span className="min-w-0 flex-1 truncate font-medium">{m.label}</span>
                      {m.id === modelId && (
                        <Check size={13} className="shrink-0 text-brand" aria-hidden="true" />
                      )}
                    </span>
                    <span className="flex items-center gap-1.5 ps-0 text-2xs text-faint">
                      {m.supports_vision && <span>vision</span>}
                      {m.supports_tool_calling && <span>tools</span>}
                      {m.supports_reasoning && <span>reasoning</span>}
                      <span className="font-mono">{m.id}</span>
                    </span>
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>

            {showProjectPicker && (
            <DropdownMenu open={projectMenuOpen} onOpenChange={resetProjectMenu}>
              <DropdownMenuTrigger asChild>
                <button
                  type="button"
                  aria-label={t("プロジェクトを選択")}
                  title={activeProject?.name ?? t("プロジェクトなし")}
                  className={cn(
                    "inline-flex h-8 max-w-48 items-center gap-1.5 rounded-md border border-line bg-surface px-2.5",
                    "text-xs font-medium text-fg",
                    "transition-colors duration-100 hover:bg-hover",
                    "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-focus",
                    "data-[state=open]:border-line-strong",
                    !activeProject && "text-faint",
                  )}
                >
                  <Folder size={13} className="shrink-0 text-muted" aria-hidden="true" />
                  <span className="min-w-0 flex-1 truncate text-start">
                    {activeProject?.name ?? t("プロジェクトなし")}
                  </span>
                  <ChevronDown size={11} className="shrink-0 text-muted" aria-hidden="true" />
                </button>
              </DropdownMenuTrigger>
              <DropdownMenuContent
                align="end"
                className={cn("min-w-56", projectMenuMode === "create" && "min-w-64")}
                onEscapeKeyDown={(e) => {
                  if (projectMenuMode === "create") {
                    e.preventDefault();
                    cancelNewProject();
                  }
                }}
              >
                {projectMenuMode === "list" ? (
                  <>
                    <DropdownMenuLabel>{t("プロジェクト")}</DropdownMenuLabel>
                    <DropdownMenuItem onSelect={() => onProjectChange?.(null)}>
                      <Folder size={13} className="text-muted" aria-hidden="true" />
                      <span className="flex-1">{t("プロジェクトなし")}</span>
                      {!projectId && <Check size={13} className="text-brand" aria-hidden="true" />}
                    </DropdownMenuItem>
                    <DropdownMenuSeparator />
                    {projects.length === 0 && (
                      <div className="px-2 py-1.5 text-2xs text-faint">
                        {t("プロジェクトがありません")}
                      </div>
                    )}
                    {projects.map((p) => (
                      <DropdownMenuItem
                        key={p.id}
                        onSelect={() => onProjectChange?.(p.id)}
                      >
                        <Folder size={13} className="text-muted" aria-hidden="true" />
                        <span className="min-w-0 flex-1 truncate">{p.name}</span>
                        {p.id === projectId && <Check size={13} className="text-brand" aria-hidden="true" />}
                      </DropdownMenuItem>
                    ))}
                    {onCreateProject && (
                      <>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem
                          onSelect={(e) => {
                            e.preventDefault();
                            setProjectMenuMode("create");
                          }}
                        >
                          <Plus size={13} className="text-muted" aria-hidden="true" />
                          <span className="flex-1">{t("新規プロジェクト")}…</span>
                        </DropdownMenuItem>
                      </>
                    )}
                  </>
                ) : (
                  <>
                    <DropdownMenuLabel className="flex items-center gap-2">
                      <button
                        type="button"
                        onClick={cancelNewProject}
                        className="inline-flex size-5 items-center justify-center rounded text-muted hover:bg-hover hover:text-fg"
                        aria-label={t("戻る")}
                        title={t("戻る")}
                      >
                        <ChevronLeft className="size-3" aria-hidden="true" />
                      </button>
                      {t("プロジェクトを作成")}
                    </DropdownMenuLabel>
                    <div className="px-1.5 pb-1.5">
                      <form
                        onSubmit={(e) => {
                          e.preventDefault();
                          void submitNewProject();
                        }}
                        className="flex flex-col gap-1.5"
                      >
                        <Input
                          ref={projectNameRef}
                          placeholder={t("プロジェクト名")}
                          value={projectDraftName}
                          onChange={(e) => setProjectDraftName(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Escape") {
                              e.preventDefault();
                              e.stopPropagation();
                              cancelNewProject();
                            }
                          }}
                          disabled={projectCreating}
                          maxLength={120}
                          aria-label={t("プロジェクト名")}
                        />
                        {projectCreateError && (
                          <div className="px-1 text-2xs text-danger">
                            {projectCreateError}
                          </div>
                        )}
                        <div className="flex items-center justify-end gap-1.5 pt-0.5">
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            onClick={cancelNewProject}
                            disabled={projectCreating}
                          >
                            {t("キャンセル")}
                          </Button>
                          <Button
                            type="submit"
                            variant="primary"
                            size="sm"
                            disabled={!projectDraftName.trim() || projectCreating}
                          >
                            {projectCreating
                              ? <Loader2 size={12} className="spin" aria-hidden="true" />
                              : <Plus size={12} aria-hidden="true" />}
                            {t("作成")}
                          </Button>
                        </div>
                      </form>
                    </div>
                  </>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
            )}

            <Button
              variant="ghost"
              size="icon"
              onClick={toggleVoiceInput}
              disabled={typeof window !== "undefined" && !("SpeechRecognition" in window || "webkitSpeechRecognition" in window)}
              title={t("音声入力")}
              aria-label={t("音声入力")}
              aria-pressed={listening}
              className={cn(listening && "relative text-danger")}
            >
              <Mic size={17} strokeWidth={1.8} className={listening ? "spin" : undefined} />
              {listening && (
                <span
                  aria-hidden="true"
                  className="absolute inset-0 -m-1 rounded-full border border-current opacity-60"
                  style={{ animation: "mix-pulse 1.2s ease-out infinite" }}
                />
              )}
            </Button>

            {busy ? (
              <Button
                variant="default"
                size="icon"
                onClick={onStop}
                aria-label={t("停止")}
                title={t("停止")}
                className="rounded-full"
              >
                <Square size={13} />
              </Button>
            ) : (
              <Button
                variant="primary"
                size="icon"
                disabled={!canSend}
                onClick={submit}
                aria-label={t("送信")}
                title={t("送信")}
                className="rounded-full"
              >
                <ArrowUp size={19} strokeWidth={2} />
              </Button>
            )}
          </div>
        </div>

        <div className="flex items-center gap-3 px-1 text-2xs text-faint">
          <span className="inline-flex items-center gap-1">
            <kbd className="rounded-sm border border-line bg-surface px-1 font-mono text-2xs leading-4">Enter</kbd>
            <span>{t("送信")}</span>
            <span className="mx-1 text-line">·</span>
            <kbd className="rounded-sm border border-line bg-surface px-1 font-mono text-2xs leading-4">Shift</kbd>
            <span>+</span>
            <kbd className="rounded-sm border border-line bg-surface px-1 font-mono text-2xs leading-4">Enter</kbd>
            <span>{t("改行")}</span>
          </span>
          <span className="flex-1" />
          {draft.length > 0 && (
            <span className="font-mono">
              {draft.length.toLocaleString()} {t("文字")}
            </span>
          )}
          {!modelId && models.length > 0 && (
            <span className="inline-flex items-center gap-1 text-warning">
              <Wrench size={11} />
              {t("モデル未設定")}
            </span>
          )}
        </div>

        {visionWarning && (
          <Alert tone="warning">
            {t("画像を添付するにはVision対応モデルを選択してください")}
          </Alert>
        )}
        {attachError && <Alert tone="danger">{attachError}</Alert>}
      </div>
    </div>
  );
});
