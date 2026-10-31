import { memo } from "react";
import { EyeOff, Wrench } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { chord } from "@/lib/platform";
import { useUi } from "@/lib/ui";
import { Alert } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Kbd } from "@/components/layout";
import { ErrorBox } from "@/components/shared";
import {
  ComposerAttachments,
  ComposerDropOverlay,
} from "./ComposerAttachments";
import { ComposerToolbar } from "./ComposerToolbar";
import { MentionPopover } from "./MentionPopover";
import { SlashCommandPopover } from "./SlashCommandPopover";
import {
  SHARED_EMPTY_MODELS,
  SHARED_EMPTY_PROJECTS,
  SHARED_EMPTY_PROVIDERS,
  useChatComposerContext,
} from "./ChatComposerContext";
import type { ComposerMode, ComposerProps } from "./composerTypes";
import { useComposerState } from "./useComposerState";

export type { DraftAttachment } from "./composerTypes";

/** Stable no-op for the provider-less fallback path (tests pass real ones). */
const NOOP_SET_MODE: (mode: ComposerMode) => void = () => {};
const NOOP_SET_MODEL: (id: string) => void = () => {};

export const Composer = memo(function Composer(props: ComposerProps) {
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
  const { busy, onSend, onStop, onSteer, draftKey = null } = props;

  const activeProject = projects.find((p) => p.id === projectId);
  const rootPath = activeProject?.rootPath ?? null;

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

  const state = useComposerState({
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
  });

  const canSteer = busy && !!onSteer && state.draft.trim().length > 0;

  const placeholder =
    mode === "agent"
      ? t("エージェントにタスクを指示… (ターミナル実行・ファイル編集が可能)")
      : mode === "thinking"
        ? t("深く考えてほしいことを入力…")
        : t("MIX-agent に話しかける…");

  return (
    <div className="border-t border-line bg-bg px-4 pb-4 pt-3">
      <div className="mx-auto flex w-full max-w-content flex-col gap-2">
        <div
          onDragOver={(e) => {
            if ([...(e.dataTransfer?.types ?? [])].includes("Files")) {
              e.preventDefault();
              state.setIsDragging(true);
            }
          }}
          onDragLeave={(e) => {
            if (e.currentTarget.contains(e.relatedTarget as Node)) return;
            state.setIsDragging(false);
          }}
          onDrop={(e) => {
            e.preventDefault();
            state.setIsDragging(false);
            const files = [...(e.dataTransfer?.files ?? [])];
            if (files.length > 0) {
              void state.handleDropFiles(files);
            }
          }}
          className={cn(
            "relative flex flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-sm",
            "transition-[border-color,box-shadow] duration-200 ease-out",
            "hover:border-line-strong",
            "focus-within:border-brand/60 focus-within:shadow-md focus-within:ring-4 focus-within:ring-brand/10",
            state.isDragging &&
              "border-brand ring-4 ring-brand/20 bg-brand-soft/20",
          )}
        >
          {state.isDragging && <ComposerDropOverlay />}
          <ComposerAttachments
            attachments={state.attachments}
            visionWarning={state.visionWarning}
            onRemove={state.removeAttachment}
          />

          <div className="relative">
            {state.mentionQuery !== null && rootPath && (
              <MentionPopover
                rootPath={rootPath}
                query={state.mentionQuery}
                onSelect={state.onSelectMention}
                onClose={() => state.setMentionQuery(null)}
                selectedIndex={state.mentionIndex}
                setSelectedIndex={state.setMentionIndex}
              />
            )}
            {state.slashQuery !== null && state.mentionQuery === null && (
              <SlashCommandPopover
                query={state.slashQuery}
                onSelect={state.onSelectSlashCommand}
                selectedIndex={state.slashIndex}
                setSelectedIndex={state.setSlashIndex}
              />
            )}
            <textarea
              ref={state.textareaRef}
              rows={1}
              aria-label={t("メッセージを入力")}
              aria-describedby="composer-hints"
              placeholder={placeholder}
              value={state.draft}
              onChange={(e) => state.onDraftChange(e.target.value)}
              onCompositionStart={state.onCompositionStart}
              onCompositionEnd={state.onCompositionEnd}
              onKeyUp={(e) => {
                if (
                  !["ArrowUp", "ArrowDown", "Enter", "Tab", "Escape"].includes(
                    e.key,
                  )
                ) {
                  state.checkMention();
                  state.checkSlash();
                }
              }}
              onClick={() => {
                state.checkMention();
                state.checkSlash();
              }}
              onPaste={(e) => {
                const files = [...(e.clipboardData?.files ?? [])].filter((f) =>
                  f.type.startsWith("image/"),
                );
                if (files.length > 0) {
                  e.preventDefault();
                  void state.addImageFiles(files);
                }
              }}
              onDrop={(e) => {
                e.preventDefault();
                state.setIsDragging(false);
                const files = [...(e.dataTransfer?.files ?? [])];
                if (files.length > 0) {
                  void state.handleDropFiles(files);
                }
              }}
              onDragOver={(e) => {
                if ([...(e.dataTransfer?.types ?? [])].includes("Files")) {
                  e.preventDefault();
                  state.setIsDragging(true);
                }
              }}
              onKeyDown={state.onKeyDown}
              className="max-h-56 min-h-16 w-full resize-none bg-transparent px-5 pt-4 pb-3 text-base leading-6 text-fg placeholder:text-faint outline-none"
            />
          </div>

          <ComposerToolbar
            mode={mode}
            setMode={setMode}
            allowedModes={allowedModes}
            models={models}
            modelId={modelId}
            setModelId={setModelId}
            providers={providers}
            projects={projects}
            projectId={projectId}
            onProjectChange={onProjectChange}
            onCreateProject={onCreateProject}
            modelsPending={modelsPending}
            projectsPending={projectsPending}
            attachmentCount={state.attachments.length}
            attaching={state.attaching}
            onAttach={state.onAttach}
            onInsertMention={state.onInsertMention}
            rootPath={rootPath}
            temporaryMode={state.temporaryMode}
            onTemporaryModeChange={state.setTemporaryMode}
            allowTools={state.allowTools}
            onAllowToolsChange={state.setAllowTools}
            listening={state.listening}
            toggleVoiceInput={state.toggleVoiceInput}
            speechSupported={state.speechSupported}
            busy={busy}
            canSteer={canSteer}
            canSend={state.canSend}
            sendDisabledReason={state.sendDisabledReason}
            sendShortcut={sendShortcut}
            onSubmit={state.onSubmit}
            onStop={onStop}
          />
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
          {state.temporaryMode && (
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
              state.draft.length === 0 && "invisible",
            )}
            aria-live="off"
            aria-hidden={state.draft.length === 0}
          >
            {state.draft.length.toLocaleString()} {t("文字")}
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

        {state.visionWarning && (
          <Alert tone="warning">
            {t("画像を添付するにはVision対応モデルを選択してください")}
          </Alert>
        )}
        {state.attachError && <ErrorBox error={state.attachError} />}
      </div>
    </div>
  );
});
