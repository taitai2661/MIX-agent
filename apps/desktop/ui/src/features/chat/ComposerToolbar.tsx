/// Composer toolbar: attach / mention / mode controls on the left, model,
/// project, voice and send/stop actions on the right.
///
/// Fixed toolbar: every control is always rendered with truncation, so the row
/// never gains or loses buttons while the window or side panels move.
/// (Container queries used to hide the mode label / project picker below
/// 18–24rem, which flickered the whole row during every panel drag.)

import { ArrowUp, AtSign, Check, Mic, Paperclip, Square } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { chord } from "@/lib/platform";
import type { SendShortcut } from "@/lib/ui";
import type { ModelView, ProjectView, ProviderView } from "@/lib/types";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
  DropdownMenuTriggerButton,
} from "@/components/ui/dropdown-menu";
import { Spinner } from "@/components/ui/spinner";
import { ALL_MODES } from "./composerModes";
import type { ComposerMode } from "./composerTypes";
import { LocalRuntimeStatusChip } from "./LocalRuntimeStatusChip";
import { ModelPicker } from "./ModelPicker";
import { ProjectMenu } from "./ProjectMenu";
import { MAX_ATTACHMENTS } from "./useAttachments";

interface ComposerToolbarProps {
  mode: ComposerMode;
  setMode: (mode: ComposerMode) => void;
  /** Modes the active surface exposes. When empty, fall back to ALL_MODES. */
  allowedModes?: ReadonlyArray<ComposerMode>;
  models: ModelView[];
  modelId: string;
  setModelId: (id: string) => void;
  providers: ProviderView[];
  projects: ProjectView[];
  projectId: string | null;
  onProjectChange?: (id: string | null) => void;
  onCreateProject?: (name: string) => Promise<string | null>;
  modelsPending: boolean;
  projectsPending: boolean;
  attachmentCount: number;
  attaching: boolean;
  onAttach: () => void | Promise<void>;
  /** Inserts `@` at the caret and opens the mention popover. */
  onInsertMention: () => void;
  /** Root of the active project; disables the mention button when absent. */
  rootPath: string | null;
  temporaryMode: boolean;
  onTemporaryModeChange: (value: boolean) => void;
  allowTools: boolean;
  onAllowToolsChange: (value: boolean) => void;
  listening: boolean;
  toggleVoiceInput: () => void;
  speechSupported: boolean;
  busy: boolean;
  canSteer: boolean;
  canSend: boolean;
  sendDisabledReason: string | null;
  sendShortcut: SendShortcut;
  onSubmit: () => void | Promise<void>;
  onStop: () => void;
}

export function ComposerToolbar({
  mode,
  setMode,
  allowedModes,
  models,
  modelId,
  setModelId,
  providers,
  projects,
  projectId,
  onProjectChange,
  onCreateProject,
  modelsPending,
  projectsPending,
  attachmentCount,
  attaching,
  onAttach,
  onInsertMention,
  rootPath,
  temporaryMode,
  onTemporaryModeChange,
  allowTools,
  onAllowToolsChange,
  listening,
  toggleVoiceInput,
  speechSupported,
  busy,
  canSteer,
  canSend,
  sendDisabledReason,
  sendShortcut,
  onSubmit,
  onStop,
}: ComposerToolbarProps) {
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

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-2 border-t border-line px-3 pt-2 pb-2.5">
      <Button
        variant="ghost"
        size="icon"
        onClick={() => void onAttach()}
        disabled={attaching || attachmentCount >= MAX_ATTACHMENTS}
        title={`${t("画像を添付")} — ${t("貼り付け・ドロップにも対応")}`}
        aria-label={t("画像を添付")}
        className="shrink-0 rounded-full"
      >
        {attaching ? <Spinner size={18} /> : <Paperclip size={18} />}
      </Button>

      <Button
        variant="ghost"
        size="icon"
        onClick={onInsertMention}
        disabled={!rootPath}
        title={
          rootPath
            ? t("ファイルをメンション (@)")
            : t("プロジェクトを選択するとファイルをメンションできます")
        }
        aria-label={
          rootPath
            ? t("ファイルをメンション (@)")
            : t("プロジェクトを選択するとファイルをメンションできます")
        }
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
          onTemporaryChange={onTemporaryModeChange}
          allowTools={allowTools}
          onAllowToolsChange={onAllowToolsChange}
          runtimeStatus={
            <LocalRuntimeStatusChip
              modelId={modelId}
              models={models}
              providers={providers}
            />
          }
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
        <span aria-hidden="true" className="mx-1 h-5 w-px shrink-0 bg-line" />

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
          <div className="flex shrink-0 items-center gap-1.5">
            {canSteer && (
              <Button
                variant="primary"
                size="icon"
                onClick={() => void onSubmit()}
                aria-label={t("実行中に追加指示を送る")}
                title={t("実行中に追加指示を送る")}
                className="rounded-full shadow-md"
              >
                <ArrowUp size={18} />
              </Button>
            )}
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
          </div>
        ) : (
          <Button
            variant="primary"
            size="icon"
            disabled={!canSend}
            onClick={() => void onSubmit()}
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
  );
}
