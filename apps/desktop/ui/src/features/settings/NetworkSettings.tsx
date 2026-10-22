import { Globe } from "lucide-react";
import { t } from "@/lib/i18n";
import { Card, Field, Section } from "@/components/layout";
import { Textarea } from "@/components/ui/input";
import { useSettingsForm } from "@/lib/settings";

export function NetworkSettings() {
  const { data, patch } = useSettingsForm();

  return (
    <Section>
      <Card title={t("Egress 許可リスト")} icon={<Globe size={13} />}>
        <Field
          hint={t(
            "未設定時は Tool がすべての公開ドメインへ到達可能。ワイルドカード (*) と完全一致ドメインを混在可。",
          )}
        >
          <Textarea
            placeholder="example.com, *.openai.com"
            value={data.egress_allowlist.join("\n")}
            onChange={(e) =>
              patch({
                egress_allowlist: e.target.value.split(/\r?\n/).filter(Boolean),
              })
            }
          />
        </Field>
      </Card>
    </Section>
  );
}
