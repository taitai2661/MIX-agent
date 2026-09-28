import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Zap } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import type { SettingsView } from "@/lib/types";
import { Card, Field, Section } from "@/components/layout";
import { Input, Select } from "@/components/ui/input";

const FALLBACK: SettingsView = {
  default_model_id: null,
  default_mode: null,
  egress_allowlist: [],
  auto_update: true,
  telemetry_consent: false,
  language: null,
  theme: null,
};

export function GeneralSettings() {
  const queryClient = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: api.getSettings });
  const data: SettingsView = settings.data ?? FALLBACK;
  const patch = (next: Partial<SettingsView>) =>
    queryClient.setQueryData<SettingsView>(["settings"], { ...data, ...next });

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
          <Field label="Mode">
            <Select
              value={data.default_mode ?? "chat"}
              onChange={(e) => patch({ default_mode: e.target.value })}
            >
              <option value="chat">Chat</option>
              <option value="thinking">Thinking</option>
              <option value="agent">Agent</option>
            </Select>
          </Field>
        </div>
      </Card>
    </Section>
  );
}
