import type { Row } from "@/app/api";
import { t } from "@/app/i18n";
import { Button } from "@/components/button";
import { ModelPicker } from "@/components/model-picker";
import { SelectMenu } from "@/components/select-menu";
import { ErrorBox } from "@/components/shared";
import { Toggle } from "@/components/toggle";
import {
  ArrowUp,
  Bot,
  Brain,
  FileText,
  FolderKanban,
  MessageSquare,
  Paperclip,
  ShieldCheck,
  SlidersHorizontal,
  Square,
  UserCog,
  X,
} from "lucide-react";
import { useEffect, useRef, type FormEvent, type RefObject } from "react";

import { ModeStatus } from "./ModeStatus";
import type {
  Artifact,
  ChatAgent,
  ChatMode,
  ChatModel,
  PermissionRule,
  Run,
  Tool,
} from "./types";

const modes = [
  "chat",
  "thinking",
  "agent",
] as const satisfies readonly ChatMode[];

export type ChatComposerProps = {
  text: string;
  onTextChange: (value: string) => void;
  onSubmit: (event?: FormEvent<HTMLFormElement>) => void;
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  attachments: Artifact[];
  onRemoveAttachment: (artifact: Artifact) => void;
  onAttach: (files: File[]) => void;
  uploading: boolean;
  models?: ChatModel[];
  agents?: ChatAgent[];
  projects?: Row[];
  tools?: Tool[];
  permissionRules?: PermissionRule[];
  model: string;
  onModelChange: (id: string) => void;
  mode: ChatMode;
  onModeChange: (mode: ChatMode) => void;
  researchMode: boolean;
  onResearchModeChange: (value: boolean) => void;
  agent: string;
  onAgentChange: (id: string) => void;
  selectedProject: string;
  onSelectedProjectChange: (id: string) => void;
  conversationId?: string;
  temporaryMode: boolean;
  onTemporaryModeChange: (value: boolean) => void;
  allowTools: boolean;
  onAllowToolsChange: (value: boolean) => void;
  active: boolean;
  busy: boolean;
  needsProviderSetup: boolean;
  hasNoProviders: boolean;
  pageError: unknown;
  onRetry: () => void;
  retryLabel?: string;
  onConnectProvider: () => void;
  onStop: () => void;
  run?: Run;
};

/** The composer form plus its footer; send/stop and model settings live here. */
export function ChatComposer({
  text,
  onTextChange,
  onSubmit,
  textareaRef,
  attachments,
  onRemoveAttachment,
  onAttach,
  uploading,
  models,
  agents,
  projects,
  tools,
  permissionRules,
  model,
  onModelChange,
  mode,
  onModeChange,
  researchMode,
  onResearchModeChange,
  agent,
  onAgentChange,
  selectedProject,
  onSelectedProjectChange,
  conversationId,
  temporaryMode,
  onTemporaryModeChange,
  allowTools,
  onAllowToolsChange,
  active,
  busy,
  needsProviderSetup,
  hasNoProviders,
  pageError,
  onRetry,
  retryLabel,
  onConnectProvider,
  onStop,
  run,
}: ChatComposerProps) {
  const fileRef = useRef<HTMLInputElement>(null);
  const selected = models?.find((x) => x.id === model);
  const selectedAgent = agents?.find((a) => a.id === agent);
  const modeLabel = mode;
  const modelLabel =
    model === "auto"
      ? "Auto"
      : selected?.data.name || selected?.data.model_id || t("モデルを選択");
  const permissionCounts = (tools || []).reduce(
    (counts: Record<string, number>, tool) => {
      const rule = permissionRules
        ?.filter((r) => r.data.agent_id === agent && r.data.tool_id === tool.id)
        .at(-1);
      const value = rule?.data.permission || tool.default_permission || "allow";
      counts[value] = (counts[value] || 0) + 1;
      return counts;
    },
    {},
  );

  useEffect(() => {
    const el = textareaRef.current;
    if (el) {
      el.style.height = "auto";
      el.style.height = el.scrollHeight + "px";
    }
  }, [text, textareaRef]);

  return (
    <div className="composer-wrap">
      {needsProviderSetup && (
        <div className="notice chat-setup-cta" role="status">
          <Bot size={17} />
          <span>
            {t(
              hasNoProviders
                ? "AI Providerが接続されていません。チャットを始めるにはProviderを接続してください。"
                : "利用できるモデルがありません。Providerを接続してモデルを取得してください。",
            )}
          </span>
          <Button variant="outline" onClick={onConnectProvider}>
            {t("Providerを接続")}
          </Button>
        </div>
      )}
      <ErrorBox error={pageError} onRetry={onRetry} retryLabel={retryLabel} />
      <form className="composer" onSubmit={onSubmit}>
        {temporaryMode && (
          <p className="temporary-notice" role="status">
            {t(
              "一時モード: Run終了後にMIX側の会話・回答・送信した添付を削除します。選択した添付は送信前にアップロードされます。Toolは",
            )}
            {t(
              allowTools
                ? "許可中（外部への送信や副作用が残る場合があります）"
                : "無効",
            )}
            。
          </p>
        )}
        {attachments.length > 0 && (
          <div className="attachments">
            {attachments.map((a) => (
              <span
                key={a.artifact_id}
                className={
                  a.mime?.startsWith("image/") ? "attachment-image-span" : ""
                }
              >
                {a.mime?.startsWith("image/") ? (
                  <img
                    src={`/api/v1/artifacts/${a.artifact_id}`}
                    alt=""
                    className="attachment-thumb"
                  />
                ) : (
                  <FileText size={13} />
                )}
                <span className="attachment-name">{a.name}</span>
                <button
                  type="button"
                  aria-label={t("添付解除")}
                  onClick={() => onRemoveAttachment(a)}
                >
                  <X size={12} />
                </button>
              </span>
            ))}
          </div>
        )}
        <textarea
          ref={textareaRef}
          aria-label={t("メッセージ")}
          placeholder={t("MIX agent にメッセージを送信…")}
          value={text}
          onChange={(e) => onTextChange(e.target.value)}
          onPaste={(e) => {
            const items = Array.from(e.clipboardData.items);
            const files = items
              .filter((item) => item.kind === "file")
              .map((item) => item.getAsFile())
              .filter((f): f is File => f !== null);
            if (files.length > 0) {
              e.preventDefault();
              onAttach(files);
            }
          }}
          onKeyDown={(e) => {
            if (
              e.key === "Enter" &&
              !e.shiftKey &&
              !e.nativeEvent.isComposing &&
              e.nativeEvent.keyCode !== 229
            ) {
              e.preventDefault();
              onSubmit();
            }
          }}
        />
        <div className="composer-bottom">
          <details className="assistant-settings">
            <summary>
              <SlidersHorizontal size={15} />
              <span className="assistant-settings-summary">
                {modelLabel} · {modeLabel}
              </span>
            </summary>
            <div className="assistant-settings-panel">
              <div className="assistant-panel-head">
                <span className="assistant-panel-icon">
                  <SlidersHorizontal size={15} />
                </span>
                <span className="assistant-panel-title">
                  <b>{t("アシスタント設定")}</b>
                  <small>
                    {modelLabel} · {modeLabel}
                  </small>
                </span>
              </div>
              <div className="assistant-sections">
                <section className="assistant-section">
                  <h4 className="assistant-section-title">
                    {t("モデルと応答方法")}
                  </h4>
                  <div className="assistant-setting-row">
                    <span>{t("モデル")}</span>
                    <ModelPicker
                      models={models}
                      value={model}
                      onChange={onModelChange}
                      temporaryMode={temporaryMode}
                      allowTools={allowTools}
                      onTemporaryModeChange={onTemporaryModeChange}
                      onAllowToolsChange={onAllowToolsChange}
                    />
                  </div>
                  <div className="assistant-setting-row mode-row">
                    <span>{t("応答方法")}</span>
                    <div className="mode-switch">
                      {modes.map((m) => (
                        <button
                          type="button"
                          key={m}
                          disabled={active}
                          className={mode === m ? "selected" : ""}
                          aria-pressed={mode === m}
                          onClick={() => onModeChange(m)}
                        >
                          {m === "chat" ? (
                            <MessageSquare size={13} />
                          ) : m === "thinking" ? (
                            <Brain size={13} />
                          ) : (
                            <Bot size={13} />
                          )}
                          {m}
                        </button>
                      ))}
                    </div>
                  </div>
                </section>
                <section className="assistant-section">
                  <h4 className="assistant-section-title">{t("文脈")}</h4>
                  <div className="assistant-setting-row">
                    <span>{t("プロジェクト")}</span>
                    <SelectMenu
                      value={selectedProject}
                      disabled={!!conversationId || temporaryMode}
                      icon={<FolderKanban size={14} />}
                      ariaLabel={t("プロジェクト")}
                      onChange={onSelectedProjectChange}
                      options={[
                        { value: "", label: t("なし") },
                        ...(projects || []).map((project) => ({
                          value: project.id,
                          label: project.data.name,
                        })),
                      ]}
                    />
                  </div>
                  <div className="assistant-setting-row assistant-toggle-row">
                    <span className="assistant-toggle-label">
                      <span>{t("調査モード")}</span>
                      <small id="research-mode-hint">
                        {t("Webを調べ、出典つきレポートを作成")}
                      </small>
                    </span>
                    <Toggle
                      checked={researchMode}
                      disabled={active || temporaryMode}
                      label={t("調査モード")}
                      describedBy="research-mode-hint"
                      onChange={onResearchModeChange}
                    />
                  </div>
                </section>
                <section className="assistant-section">
                  <h4 className="assistant-section-title">
                    {t("アシスタントと権限")}
                  </h4>
                  <div className="assistant-setting-row">
                    <span>{t("アシスタント")}</span>
                    <SelectMenu
                      value={agent}
                      icon={<UserCog size={14} />}
                      ariaLabel={t("アシスタント")}
                      onChange={onAgentChange}
                      options={[
                        { value: "", label: t("標準") },
                        ...(agents || []).map((a) => ({
                          value: a.id,
                          label: a.data.name,
                        })),
                      ]}
                    />
                  </div>
                  <div
                    className="assistant-permissions"
                    title={t("現在のアシスタントに適用されるTool権限")}
                  >
                    <ShieldCheck size={14} />
                    <span>
                      {t("ツール権限: 許可")} {permissionCounts.allow || 0}{" "}
                      {t("· 確認")} {permissionCounts.ask || 0} {t("· 拒否")}{" "}
                      {permissionCounts.deny || 0}
                    </span>
                  </div>
                  <ModeStatus
                    mode={mode}
                    model={selected}
                    agent={selectedAgent}
                    budget={
                      active && run?.mode === mode ? run.budget : undefined
                    }
                  />
                </section>
              </div>
            </div>
          </details>
          <div className="composer-actions">
            {uploading && (
              <span className="send-hint" role="status">
                {t("アップロード中…")}
              </span>
            )}
            <span className="send-hint">
              {t("Enterで送信 / Shift+Enterで改行")}
            </span>
            <button
              type="button"
              className="icon"
              aria-label={t("添付")}
              onClick={() => fileRef.current?.click()}
            >
              <Paperclip size={18} />
            </button>
            <input
              ref={fileRef}
              hidden
              type="file"
              multiple
              accept="image/*,.txt,.md,.pdf"
              onChange={(e) => {
                const files = Array.from(e.currentTarget.files || []);
                e.currentTarget.value = "";
                onAttach(files);
              }}
            />
            {active ? (
              <button
                type="button"
                className="send"
                aria-label={t("停止")}
                onClick={onStop}
              >
                <Square size={15} />
              </button>
            ) : (
              <button
                className="send"
                aria-label={t("送信")}
                disabled={
                  busy ||
                  uploading ||
                  needsProviderSetup ||
                  (!text.trim() && !attachments.length)
                }
              >
                <ArrowUp size={19} />
              </button>
            )}
          </div>
        </div>
      </form>
      <p className="composer-foot">
        {t(
          "MIX agent の回答には誤りが含まれることがあります。重要な情報は確認してください。",
        )}
      </p>
    </div>
  );
}
