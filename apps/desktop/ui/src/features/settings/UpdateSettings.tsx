import { RefreshCw } from "lucide-react";
import { t } from "@/lib/i18n";
import { Card, Section } from "@/components/layout";
import { ToggleRow } from "@/components/common";
import { useSettingsForm } from "@/lib/settings";

export function UpdateSettings() {
  const { data, patch } = useSettingsForm();

  return (
    <Section>
      <Card title={t("アップデート")} icon={<RefreshCw size={13} />}>
        <ToggleRow
          label={t("自動更新を有効化")}
          checked={data.auto_update}
          onChange={(auto_update) => patch({ auto_update })}
        />
        <ToggleRow
          label={t("匿名の利用統計を共有 (将来)")}
          checked={data.telemetry_consent}
          onChange={(telemetry_consent) => patch({ telemetry_consent })}
        />
      </Card>
    </Section>
  );
}
