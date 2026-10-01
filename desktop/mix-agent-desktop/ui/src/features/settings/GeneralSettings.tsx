import { Compass, Zap } from "lucide-react";
import { t } from "@/lib/i18n";
import { useOnboardingGate } from "@/features/onboarding/useOnboardingGate";
import { Card, Field, Row, Section } from "@/components/layout";
import { Input, Select } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { useSettingsForm } from "@/lib/settings";

export function GeneralSettings() {
  const { data, patch } = useSettingsForm();
  const onboarding = useOnboardingGate();

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
          <Field label={t("モデル")}>
            <Input
              placeholder={t("未設定")}
              value={data.default_model_id ?? ""}
              onChange={(e) => patch({ default_model_id: e.target.value || null })}
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
    </Section>
  );
}
