import { Zap } from "lucide-react";
import { t } from "@/lib/i18n";
import { Card, Field, Section } from "@/components/layout";
import { Input, Select } from "@/components/ui/input";
import { useSettingsForm } from "@/lib/settings";

export function GeneralSettings() {
  const { data, patch } = useSettingsForm();

  return (
    <Section>
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
