import { memo, useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  ArrowUp,
  Bot,
  Brain,
  Check,
  Eye,
  Folder,
  MessageSquare,
  Mic,
  Paperclip,
  Plus,
  Square,
  Wrench,
  X,
} from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import type { ModelView, ProjectView } from "@/lib/types";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  DropdownMenuTriggerButton,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Kbd } from "@/components/layout";
import { Spinner } from "@/components/ui/spinner";

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
  modelsPending = false,
  projectsPending = false,
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
  /** While true the model/project lists are "not known yet", not "empty". The
   *  menus used to say "No model configured — add a provider first" on every
   *  open that happened before the query settled. */
  modelsPending?: boolean;
  projectsPending?: boolean;
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
  const [projectMenuMode, setProjectMenuMode] = useState<"list" | "create">(
    "list",
  );
  const [projectDraftName, setProjectDraftName] = useState("");
  const [projectCreating, setProjectCreating] = useState(false);
  const [projectCreateError, setProjectCreateError] = useState<string | null>(
    null,
  );
  const projectNameRef = useRef<HTMLInputElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const speechRef = useRef<{ stop: () => void; abort: () => void } | null>(
    null,
  );

  const selectedModel = models.find((m) => m.id === modelId);
  // The selected model is gone from the list — its provider was deleted, or the
  // models failed to load while an id was already set. Sending would queue a run
  // against a model id no provider can serve, and the failure only surfaces
  // after the run starts, so it is blocked here instead. The `models.length > 0`
  // guard keeps an empty list from reading as "stale" on first load, which would
  // hide the real state (nothing is configured yet).
  const staleModel = !!modelId && models.length > 0 && !selectedModel;
  // Narrow the master mode list down to what the active surface allows. When
  // the surface only allows one mode we hide the dropdown entirely below.
  const MODES =
    allowedModes && allowedModes.length > 0
      ? ALL_MODES.filter((m) => allowedModes.includes(m.id))
      : ALL_MODES;
  const activeMode = MODES.find((m) => m.id === mode) ?? MODES[0];
  const activeProject = projects.find((p) => p.id === projectId) ?? null;
  // Project context only matters for the agent surface; chat surface hides
  // the picker to keep the toolbar minimal.
  const showProjectPicker = MODES.some((m) => m.id === "agent");
  const hasImage = attachments.some((a) => a.mime_type.startsWith("image/"));
  const visionWarning =
    hasImage && selectedModel != null && !selectedModel.supports_vision;
  const canSend =
    draft.trim().length > 0 &&
    !busy &&
    !!modelId &&
    !staleModel &&
    (!hasImage || !selectedModel || selectedModel.supports_vision);

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

  useEffect(() => () => speechRef.current?.abort(), []);

  // Welcome chips (or external callers) can pre-fill the draft via this event
  // instead of having to thread state through ChatPane.
  useEffect(() => {
    const onInsert = (event: Event) => {
      const detail = (
        event as CustomEvent<{ text?: string; mode?: typeof mode }>
      ).detail;
      if (typeof detail?.text === "string") setDraft(detail.text);
      // A greeting picked on one surface may name a mode the current surface
      // does not expose (e.g. "thinking" after switching to Desktop-Agent), so
      // only apply a mode that is actually allowed here.
      if (detail?.mode) {
        const allowed =
          allowedModes && allowedModes.length > 0
            ? allowedModes
            : ALL_MODES.map((m) => m.id);
        if ((allowed as readonly string[]).includes(detail.mode))
          setMode(detail.mode);
      }
      window.requestAnimationFrame(() => textareaRef.current?.focus());
    };
    window.addEventListener("mix:insert-prompt", onInsert);
    return () => window.removeEventListener("mix:insert-prompt", onInsert);
  }, [setMode, allowedModes]);

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
      onresult:
        | ((event: {
            resultIndex: number;
            results: ArrayLike<
              ArrayLike<{ transcript: string; isFinal: boolean }>
            >;
          }) => void)
        | null;
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
    const RecognitionCtor =
      (window as RecognitionWindow).SpeechRecognition ??
      (window as RecognitionWindow).webkitSpeechRecognition;
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
      setDraft(
        [initialDraft, spoken, interim]
          .filter(Boolean)
          .join(initialDraft ? " " : ""),
      );
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

  const placeholder =
    mode === "agent"
      ? t("複雑な作業を依頼…")
      : mode === "thinking"
        ? t("深く考えてほしいことを入力…")
        : t("MIX-agent に話しかける…");

  return (
    // A container query, not a breakpoint: what decides whether the toolbar
    // fits is the width of the chat column, which follows the side panels the
    // user has open — the window can be 1200px wide with the centre column
    // squeezed to 300 by the file tree and the activity panel.
    <div className="@container border-t border-line bg-bg px-6 pb-5 pt-3.5">
      <div className="mx-auto flex w-full max-w-content flex-col gap-2">
        <div
          className={cn(
            "flex flex-col overflow-hidden rounded-2xl border border-line bg-surface",
            "transition-[border-color,box-shadow] duration-150 ease-out",
            "focus-within:border-brand focus-within:ring-2 focus-within:ring-brand/15",
          )}
        >
          {attachments.length > 0 && (
            <div className="flex flex-wrap gap-1.5 border-b border-line px-3 py-2">
              {attachments.map((a, i) => (
                <span
                  key={`${a.filename}-${i}`}
                  className={cn(
                    "inline-flex items-center gap-1.5 rounded-md border border-line bg-inset py-0.5 ps-1.5 pe-1 text-xs text-muted",
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
                    onClick={() =>
                      setAttachments((prev) => prev.filter((_, j) => j !== i))
                    }
                    aria-label={`${a.filename} ${t("削除")}`}
                  >
                    <X size={11} />
                  </Button>
                </span>
              ))}
              <span className="self-center text-xs text-faint">
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
              if (
                e.key === "Enter" &&
                !e.shiftKey &&
                !e.nativeEvent.isComposing
              ) {
                e.preventDefault();
                submit();
              }
            }}
            className="max-h-56 min-h-14 w-full resize-none bg-transparent px-4 py-3.5 text-sm leading-6 text-fg placeholder:text-faint outline-none"
          />

          {/* Everything in this row is a control the user still needs at narrow widths,
              so each one is allowed to give up width rather than push the row
              out of the composer — the model and project names truncate, the
              icon buttons are `shrink-0` and the middle spacer absorbs the
              rest. */}
          <div className="flex items-center gap-1 px-2.5 pb-2.5">
            <Button
              variant="ghost"
              size="icon"
              onClick={() => void onAttach()}
              disabled={attaching || attachments.length >= MAX_ATTACHMENTS}
              title={t("画像を添付")}
              aria-label={t("画像を添付")}
            >
              {attaching ? (
                <Spinner size={17} />
              ) : (
                <Paperclip size={17} strokeWidth={1.8} />
              )}
            </Button>

            {MODES.length > 1 ? (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <DropdownMenuTriggerButton
                    aria-label={t("モード")}
                    title={t(activeMode.label)}
                    className="shrink border-transparent bg-transparent hover:bg-hover"
                  >
                    <activeMode.Icon
                      size={13}
                      className="shrink-0 text-muted"
                      aria-hidden="true"
                    />
                    {/* The word goes before the width does: the icon still says
                        "agent", and the label only fits from 18rem up. */}
                    <span className="hidden min-w-0 truncate @min-[18rem]:inline">
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
                className="inline-flex h-control min-w-0 shrink items-center gap-1.5 rounded-md px-2.5 text-xs font-medium text-fg"
              >
                <activeMode.Icon
                  size={13}
                  className="shrink-0 text-muted"
                  aria-hidden="true"
                />
                <span className="hidden min-w-0 truncate @min-[18rem]:inline">
                  {t(activeMode.label)}
                </span>
              </span>
            )}

            <span className="min-w-0 flex-1" />

            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <DropdownMenuTriggerButton
                  aria-label={t("モデル")}
                  title={
                    selectedModel?.label ??
                    (models.length === 0 && !modelsPending
                      ? t("モデル未設定")
                      : "")
                  }
                  className="border-transparent bg-transparent hover:bg-hover"
                >
                  <span className="min-w-0 flex-1 truncate text-start">
                    {selectedModel?.label ??
                      (modelsPending
                        ? t("読み込み中…")
                        : models.length === 0
                          ? t("モデル未設定")
                          : t("モデル"))}
                  </span>
                  {selectedModel?.supports_vision && (
                    <Eye
                      size={11}
                      className="shrink-0 text-faint"
                      aria-hidden="true"
                    />
                  )}
                </DropdownMenuTriggerButton>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="min-w-64">
                <DropdownMenuLabel>{t("モデル")}</DropdownMenuLabel>
                {modelsPending ? (
                  <div className="px-2 py-1.5 text-xs text-faint">
                    {t("読み込み中…")}
                  </div>
                ) : (
                  models.length === 0 && (
                    <div className="px-2 py-1.5 text-xs text-faint">
                      {t(
                        "モデルが未設定です。先にプロバイダーを追加してください。",
                      )}
                    </div>
                  )
                )}
                {models.map((m) => (
                  <DropdownMenuItem
                    key={m.id}
                    onSelect={() => setModelId(m.id)}
                    description={
                      <span className="flex items-center gap-1.5">
                        {m.supports_vision && <span>vision</span>}
                        {m.supports_tool_calling && <span>tools</span>}
                        {m.supports_reasoning && <span>reasoning</span>}
                        <span className="font-mono">{m.id}</span>
                      </span>
                    }
                  >
                    <span className="min-w-0 flex-1 truncate font-medium">
                      {m.label}
                    </span>
                    {m.id === modelId && (
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

            {showProjectPicker && (
              // `contents` keeps the trigger a direct flex item of the row, so
              // hiding the picker below 24rem does not change the gap it sits in.
              <span className="hidden @min-[24rem]:contents">
                <DropdownMenu
                  open={projectMenuOpen}
                  onOpenChange={resetProjectMenu}
                >
                  <DropdownMenuTrigger asChild>
                    <DropdownMenuTriggerButton
                      aria-label={t("プロジェクトを選択")}
                      title={activeProject?.name ?? t("プロジェクトなし")}
                      className={cn(
                        "max-w-48 min-w-0 shrink border-transparent bg-transparent hover:bg-hover",
                        !activeProject && !projectsPending && "text-faint",
                      )}
                    >
                      <Folder
                        size={13}
                        className="shrink-0 text-muted"
                        aria-hidden="true"
                      />
                      <span className="min-w-0 flex-1 truncate text-start">
                        {activeProject?.name ?? t("プロジェクトなし")}
                      </span>
                    </DropdownMenuTriggerButton>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent
                    align="end"
                    className={cn(
                      "min-w-56",
                      projectMenuMode === "create" && "min-w-64",
                    )}
                    onEscapeKeyDown={(e) => {
                      if (projectMenuMode === "create") {
                        e.preventDefault();
                        cancelNewProject();
                      }
                    }}
                  >
                    {projectMenuMode === "list" ? (
                      <>
                        <DropdownMenuLabel>
                          {t("プロジェクト")}
                        </DropdownMenuLabel>
                        <DropdownMenuItem
                          onSelect={() => onProjectChange?.(null)}
                        >
                          <Folder
                            size={13}
                            className="text-muted"
                            aria-hidden="true"
                          />
                          <span className="flex-1">
                            {t("プロジェクトなし")}
                          </span>
                          {!projectId && (
                            <Check
                              size={13}
                              className="text-brand"
                              aria-hidden="true"
                            />
                          )}
                        </DropdownMenuItem>
                        <DropdownMenuSeparator />
                        {projectsPending ? (
                          <div className="px-2 py-1.5 text-xs text-faint">
                            {t("読み込み中…")}
                          </div>
                        ) : (
                          projects.length === 0 && (
                            <div className="px-2 py-1.5 text-xs text-faint">
                              {t("プロジェクトがありません")}
                            </div>
                          )
                        )}
                        {projects.map((p) => (
                          <DropdownMenuItem
                            key={p.id}
                            onSelect={() => onProjectChange?.(p.id)}
                          >
                            <Folder
                              size={13}
                              className="text-muted"
                              aria-hidden="true"
                            />
                            <span className="min-w-0 flex-1 truncate">
                              {p.name}
                            </span>
                            {p.id === projectId && (
                              <Check
                                size={13}
                                className="text-brand"
                                aria-hidden="true"
                              />
                            )}
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
                              <Plus
                                size={13}
                                className="text-muted"
                                aria-hidden="true"
                              />
                              <span className="flex-1">
                                {t("新規プロジェクト")}…
                              </span>
                            </DropdownMenuItem>
                          </>
                        )}
                      </>
                    ) : (
                      <>
                        <DropdownMenuLabel>
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
                              onChange={(e) =>
                                setProjectDraftName(e.target.value)
                              }
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
                              <div className="px-1 text-xs text-danger">
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
                                disabled={
                                  !projectDraftName.trim() || projectCreating
                                }
                              >
                                {projectCreating ? (
                                  <Spinner size={12} />
                                ) : (
                                  <Plus size={12} aria-hidden="true" />
                                )}
                                {t("作成")}
                              </Button>
                            </div>
                          </form>
                        </div>
                      </>
                    )}
                  </DropdownMenuContent>
                </DropdownMenu>
              </span>
            )}

            <Button
              variant="ghost"
              size="icon"
              onClick={toggleVoiceInput}
              disabled={
                typeof window !== "undefined" &&
                !(
                  "SpeechRecognition" in window ||
                  "webkitSpeechRecognition" in window
                )
              }
              title={t("音声入力")}
              aria-label={t("音声入力")}
              className={cn(listening && "relative")}
            >
              <Mic size={17} strokeWidth={1.8} />
              {listening && (
                <span
                  aria-hidden="true"
                  className="pointer-events-none absolute inset-0 rounded-full border border-current opacity-60 animate-mix-pulse"
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

        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 px-1 text-xs text-faint">
          <span className="inline-flex min-w-0 items-center gap-1">
            <Kbd>Enter</Kbd>
            <span>{t("送信")}</span>
            <span className="mx-1 text-line">·</span>
            <Kbd>⇧</Kbd>
            <span>+</span>
            <Kbd>Enter</Kbd>
            <span>{t("改行")}</span>
          </span>
          <span className="min-w-0 flex-1" />
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
          {staleModel && (
            <span className="inline-flex items-center gap-1 text-danger">
              <Wrench size={11} />
              {t("選択したモデルが利用できません")}
            </span>
          )}
        </div>

        {staleModel && (
          <Alert tone="danger">
            {t(
              "選択したモデルのプロバイダーが削除されています。モデルを再選択してください。",
            )}
          </Alert>
        )}

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
