import { Brain, Compass, Zap } from "lucide-react";
import { t } from "@/lib/i18n";
import { useOnboardingGate } from "@/features/onboarding/useOnboardingGate";
import { Card, Field, Row, Section } from "@/components/layout";
import { ToggleRow } from "@/components/common";
import { Select } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { ModelPicker } from "@/features/chat/ModelPicker";
import { useModels, useProviders } from "@/lib/queries";
import { useSettingsForm } from "@/lib/settings";

export function GeneralSettings() {
  const { data, patch } = useSettingsForm();
  const onboarding = useOnboardingGate();
  const models = useModels();
  const providers = useProviders();

  return (
    <Section>
      <Card title={t("セットアップガイド")} icon={<Compass size={13} />}>
        <Row align="between">
          <p className="m-0 max-w-md text-xs text-muted">
            {t("モデル提供元の追加方法と、作業フォルダの選び方を確認できます。")}
          </p>
          <Button size="sm" onClick={onboarding.reopen}>
            <Compass size={12} /> {t("もう一度見る")}
          </Button>
        </Row>
      </Card>

      <Card title={t("初期設定")} icon={<Zap size={13} />}>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label={t("モデル")} hint={t("新しいスレッドで既定で使うモデル")}>
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
    </Section>
  );
}
