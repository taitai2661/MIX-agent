import { Brain, Compass, Keyboard, Lock, Zap } from "lucide-react";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { useOnboardingGate } from "@/features/onboarding/useOnboardingGate";
import { Card, Field, Row, Section } from "@/components/layout";
import { ToggleRow } from "@/components/common";
import { Select } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";
import { ModelPicker } from "@/features/chat/ModelPicker";
import { useModels, useProviders } from "@/lib/queries";
import { useSettingsForm } from "@/lib/settings";

export function GeneralSettings() {
  const { data, patch } = useSettingsForm();
  const onboarding = useOnboardingGate();
  const models = useModels();
  const providers = useProviders();
  const sendShortcut = useUi((s) => s.sendShortcut);
  const setSendShortcut = useUi((s) => s.setSendShortcut);

  return (
    <Section>
      <Card title={t("セットアップガイド")} icon={<Compass size={13} />}>
        <Row align="between">
          <p className="m-0 max-w-md text-xs text-muted">
            {t(
              "モデル提供元の追加方法と、作業フォルダの選び方を確認できます。",
            )}
          </p>
          <Button size="sm" onClick={onboarding.reopen}>
            <Compass size={12} /> {t("もう一度見る")}
          </Button>
        </Row>
      </Card>

      <Card title={t("初期設定")} icon={<Zap size={13} />}>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field
            label={t("モデル")}
            hint={t("新しいスレッドで既定で使うモデル")}
          >
            <ModelPicker
              models={models.data ?? []}
              value={data.default_model_id ?? ""}
              onChange={(id) => patch({ default_model_id: id })}
              providers={providers.data ?? []}
              pending={models.isPending}
              className="w-full max-w-none"
            />
          </Field>
          <Field label={t("モード")}>
            <Select
              value={data.default_mode ?? "chat"}
              onChange={(e) => patch({ default_mode: e.target.value })}
            >
              <option value="chat">{t("チャット")}</option>
              <option value="thinking">{t("思考")}</option>
              <option value="agent">{t("エージェント")}</option>
            </Select>
          </Field>
        </div>
      </Card>

      <Card title={t("メッセージ送信キー")} icon={<Keyboard size={13} />}>
        <div className="flex flex-col gap-2">
          <p className="m-0 text-xs text-muted">
            {t("チャット入力欄でのメッセージ送信ショートカットを設定します。")}
          </p>
          <Segmented
            aria-label={t("メッセージ送信キー")}
            className="w-full max-w-md"
          >
            <SegmentedOption
              value="enter"
              label={t("Enter で送信 (Shift+Enter で改行)")}
              selected={sendShortcut === "enter"}
              onSelect={() => setSendShortcut("enter")}
            >
              {t("Enter で送信 (Shift+Enter で改行)")}
            </SegmentedOption>
            <SegmentedOption
              value="modifier"
              label={t("⌘ / Ctrl + Enter で送信 (Enter で改行)")}
              selected={sendShortcut === "modifier"}
              onSelect={() => setSendShortcut("modifier")}
            >
              {t("⌘ / Ctrl + Enter で送信 (Enter で改行)")}
            </SegmentedOption>
          </Segmented>
        </div>
      </Card>

      <Card title={t("メモリ")} icon={<Brain size={13} />}>
        <ToggleRow
          label={t("会話から自動でメモリを作成")}
          hint={t(
            "応答のあとに、事実・好み・制約だけを抽出して保存します。「覚えて」「忘れて」の指示は常に即時処理されます。",
          )}
          checked={data.memory_auto_formation}
          onChange={(memory_auto_formation) => patch({ memory_auto_formation })}
        />
        <ToggleRow
          label={t("類似するメモリ同士を自動でリンクする")}
          hint={t(
            "抽出後に埋め込みの近傍を調べ、関連する Memory をリンクします。リンクは想起のたびに1ホップ展開されます。",
          )}
          checked={data.memory_link_embedding_neighbors}
          onChange={(memory_link_embedding_neighbors) =>
            patch({ memory_link_embedding_neighbors })
          }
        />
        <p className="m-0 text-xs text-muted">{t("既定はオフです。")}</p>
      </Card>

      <Card title={t("データベースの暗号化")} icon={<Lock size={13} />}>
        <ToggleRow
          label={t("保存時の暗号化")}
          hint={t(
            "ローカルデータベースの暗号化を有効にします。既定はオフ（Codex互換の平文SQLite）です。",
          )}
          checked={data.database_encryption}
          onChange={(database_encryption) => patch({ database_encryption })}
        />
        <p className="m-0 text-xs text-muted">
          {t("マスターキーはOSのKeychainにあります")}
        </p>
      </Card>
    </Section>
  );
}
