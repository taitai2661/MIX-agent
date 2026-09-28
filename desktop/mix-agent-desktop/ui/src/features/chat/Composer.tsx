import { useEffect, useRef, useState } from "react";
import { ArrowUp, ChevronDown, Folder, Loader2, Mic, Paperclip, Plus, ShieldCheck, Square, TriangleAlert, X } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import type { ModelView, ProjectView } from "@/lib/types";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";

export interface DraftAttachment {
  kind: string;
  mime_type: string;
  data_base64: string;
  filename: string;
}

export function Composer({
  mode,
  setMode,
  models,
  modelId,
  setModelId,
  projects = [],
  projectId = null,
  onProjectChange,
  busy,
  onSend,
  onStop,
}: {
  mode: "chat" | "thinking" | "agent";
  setMode: (mode: "chat" | "thinking" | "agent") => void;
  models: ModelView[];
  modelId: string;
  setModelId: (id: string) => void;
  projects?: ProjectView[];
  projectId?: string | null;
  onProjectChange?: (id: string | null) => void;
  busy: boolean;
  onSend: (content: string, attachments: DraftAttachment[]) => void;
  onStop: () => void;
}) {
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState<DraftAttachment[]>([]);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [attaching, setAttaching] = useState(false);
  const [listening, setListening] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const speechRef = useRef<{ stop: () => void; abort: () => void } | null>(null);

  const selectedModel = models.find((m) => m.id === modelId);
  const hasImage = attachments.some((a) => a.mime_type.startsWith("image/"));
  const visionWarning = hasImage && selectedModel != null && !selectedModel.supports_vision;
  const canSend =
    draft.trim().length > 0 && !busy && !!modelId && (!hasImage || !selectedModel || selectedModel.supports_vision);

  // Auto-grow: reset then stretch to content, capped by CSS max-height.
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [draft]);

  useEffect(() => () => speechRef.current?.abort(), []);

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
      for (const path of paths) {
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

  const placeholder =
    mode === "agent"
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
          <label className="flex items-center gap-1.5 border-b border-line px-3 py-1.5 text-muted">
            <Folder size={13} aria-hidden="true" className="shrink-0 text-faint" />
            <select
              value={projectId ?? ""}
              onChange={(event) => onProjectChange?.(event.target.value || null)}
              aria-label={t("プロジェクトを選択")}
              className="min-w-0 flex-1 cursor-pointer appearance-none truncate bg-transparent text-xs text-fg outline-none"
            >
              <option value="">{t("プロジェクトを選択")}</option>
              {projects.map((project) => <option key={project.id} value={project.id}>{project.name}</option>)}
            </select>
            <ChevronDown size={13} aria-hidden="true" className="shrink-0 text-faint" />
          </label>

          {attachments.length > 0 && (
            <div className="flex flex-wrap gap-1.5 border-b border-line px-3 py-2">
              {attachments.map((a, i) => (
                <span
                  key={`${a.filename}-${i}`}
                  className="inline-flex items-center gap-1.5 rounded-md border border-line bg-inset py-0.5 pl-1.5 pr-1 text-xs text-muted"
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
                      className="text-warning"
                      title={t("このモデルは画像入力に対応していません")}
                      aria-label={t("このモデルは画像入力に対応していません")}
                    >
                      <TriangleAlert size={12} />
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
            </div>
          )}

          <textarea
            ref={textareaRef}
            rows={1}
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
              disabled={attaching}
              title={t("画像を添付")}
              aria-label={t("画像を添付")}
            >
              {attaching
                ? <Loader2 size={17} className="spin" />
                : <Plus size={18} strokeWidth={1.8} />}
            </Button>

            <div
              role="tablist"
              aria-label="Mode"
              className="inline-flex items-center gap-0.5 rounded-lg bg-inset p-0.5"
            >
              {(["chat", "thinking", "agent"] as const).map((m) => (
                <button
                  key={m}
                  type="button"
                  role="tab"
                  aria-selected={mode === m}
                  onClick={() => setMode(m)}
                  className={cn(
                    "inline-flex h-6 items-center gap-1 rounded-md px-2 text-xs font-medium",
                    "text-muted transition-colors duration-100 hover:text-fg",
                    "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-focus",
                    mode === m && "bg-surface text-fg shadow-xs",
                  )}
                >
                  {m === "agent" && <ShieldCheck size={13} aria-hidden="true" />}
                  {m === "chat" ? "Chat" : m === "thinking" ? "Thinking" : "Agent"}
                </button>
              ))}
            </div>

            <span className="flex-1" />

            <label className="flex items-center gap-1 rounded-md px-1.5 py-1 text-xs text-muted transition-colors hover:bg-hover">
              <select
                value={modelId}
                onChange={(e) => setModelId(e.target.value)}
                aria-label="Model"
                title={models.find((m) => m.id === modelId)?.label ?? (models.length === 0 ? t("モデル未設定") : "")}
                className="max-w-44 cursor-pointer appearance-none truncate bg-transparent text-xs text-fg outline-none"
              >
                {models.map((m) => (
                  <option key={m.id} value={m.id}>{m.label}</option>
                ))}
                {models.length === 0 && <option value="">{t("モデル未設定")}</option>}
              </select>
              <ChevronDown size={13} aria-hidden="true" className="shrink-0 text-faint" />
            </label>

            <Button
              variant="ghost"
              size="icon"
              onClick={toggleVoiceInput}
              disabled={typeof window !== "undefined" && !("SpeechRecognition" in window || "webkitSpeechRecognition" in window)}
              title={t("音声入力")}
              aria-label={t("音声入力")}
              aria-pressed={listening}
              className={listening ? "text-danger" : undefined}
            >
              <Mic size={17} strokeWidth={1.8} />
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

        {visionWarning && (
          <Alert tone="warning">
            {t("画像を添付するにはVision対応モデルを選択してください")}
          </Alert>
        )}
        {attachError && <Alert tone="danger">{attachError}</Alert>}
      </div>
    </div>
  );
}
