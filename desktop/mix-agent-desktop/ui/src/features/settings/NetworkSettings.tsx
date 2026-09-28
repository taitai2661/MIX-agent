import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Globe } from "lucide-react";
import { api } from "@/lib/api";
import type { SettingsView } from "@/lib/types";
import { Card, Field, Section } from "@/components/layout";
import { Textarea } from "@/components/ui/input";

const FALLBACK: SettingsView = {
  default_model_id: null,
  default_mode: null,
  egress_allowlist: [],
  auto_update: true,
  telemetry_consent: false,
  language: null,
  theme: null,
};

export function NetworkSettings() {
  const queryClient = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: api.getSettings });
  const data: SettingsView = settings.data ?? FALLBACK;
  const patch = (next: Partial<SettingsView>) =>
    queryClient.setQueryData<SettingsView>(["settings"], { ...data, ...next });

  return (
    <Section>
      <Card title="Egress Allowlist" icon={<Globe size={13} />}>
        <Field hint="未設定時は Tool がすべての公開ドメインへ到達可能。ワイルドカード (*) と完全一致ドメインを混在可。">
          <Textarea
            placeholder="example.com, *.openai.com"
            value={data.egress_allowlist.join("\n")}
            onChange={(e) =>
              patch({ egress_allowlist: e.target.value.split(/\r?\n/).filter(Boolean) })
            }
          />
        </Field>
      </Card>
    </Section>
  );
}
