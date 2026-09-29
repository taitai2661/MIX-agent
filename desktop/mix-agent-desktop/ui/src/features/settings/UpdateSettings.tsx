import { RefreshCw } from "lucide-react";
import { t } from "@/lib/i18n";
import { Card, Section } from "@/components/layout";
import { Switch } from "@/components/ui/switch";
import { useSettingsForm } from "@/lib/settings";

function Toggle({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label className="flex items-center justify-between gap-4 text-sm text-fg">
      <span>{label}</span>
      <Switch checked={checked} onCheckedChange={onChange} />
    </label>
  );
}

export function UpdateSettings() {
  const { data, patch } = useSettingsForm();

  return (
    <Section>
      <Card title={t("アップデート")} icon={<RefreshCw size={13} />}>
        <Toggle
          label={t("自動更新を有効化")}
          checked={data.auto_update}
          onChange={(auto_update) => patch({ auto_update })}
        />
        <Toggle
          label={t("匿名の利用統計を共有 (将来)")}
          checked={data.telemetry_consent}
          onChange={(telemetry_consent) => patch({ telemetry_consent })}
        />
      </Card>
    </Section>
  );
}
