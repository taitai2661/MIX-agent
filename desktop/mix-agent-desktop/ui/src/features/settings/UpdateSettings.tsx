import { useQuery, useQueryClient } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";
import { api } from "@/lib/api";
import type { SettingsView } from "@/lib/types";
import { Card, Section } from "@/components/layout";
import { Switch } from "@/components/ui/switch";

const FALLBACK: SettingsView = {
  default_model_id: null,
  default_mode: null,
  egress_allowlist: [],
  auto_update: true,
  telemetry_consent: false,
  language: null,
  theme: null,
};

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
  const queryClient = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: api.getSettings });
  const data: SettingsView = settings.data ?? FALLBACK;
  const patch = (next: Partial<SettingsView>) =>
    queryClient.setQueryData<SettingsView>(["settings"], { ...data, ...next });

  return (
    <Section>
      <Card title="Updates" icon={<RefreshCw size={13} />}>
        <Toggle
          label="自動更新を有効化"
          checked={data.auto_update}
          onChange={(auto_update) => patch({ auto_update })}
        />
        <Toggle
          label="匿名の利用統計を共有 (将来)"
          checked={data.telemetry_consent}
          onChange={(telemetry_consent) => patch({ telemetry_consent })}
        />
      </Card>
    </Section>
  );
}
