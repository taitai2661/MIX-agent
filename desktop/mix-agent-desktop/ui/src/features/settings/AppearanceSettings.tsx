import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Monitor, Moon, Palette, Sun, Zap } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import type { Theme } from "@/lib/theme";
import type { SettingsView } from "@/lib/types";
import { Card, Row, Section } from "@/components/layout";
import { Button } from "@/components/ui/button";

const FALLBACK: SettingsView = {
  default_model_id: null,
  default_mode: null,
  egress_allowlist: [],
  auto_update: true,
  telemetry_consent: false,
  language: null,
  theme: null,
};

const THEMES: Array<{ value: Theme; label: string; Icon: typeof Sun }> = [
  { value: "light", label: "ライト", Icon: Sun },
  { value: "dark", label: "ダーク", Icon: Moon },
  { value: "system", label: "システムに従う", Icon: Monitor },
];

export function AppearanceSettings() {
  const queryClient = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: api.getSettings });
  const setTheme = useUi((s) => s.setTheme);
  const setLocale = useUi((s) => s.setLocale);
  const data: SettingsView = settings.data ?? FALLBACK;
  const patch = (next: Partial<SettingsView>) =>
    queryClient.setQueryData<SettingsView>(["settings"], { ...data, ...next });

  return (
    <Section>
      <Card title={t("テーマ")} icon={<Palette size={13} />}>
        <Row className="gap-1.5">
          {THEMES.map(({ value, label, Icon }) => {
            const active = (data.theme ?? "system") === value;
            return (
              <Button
                key={value}
                size="sm"
                aria-pressed={active}
                onClick={() => {
                  patch({ theme: value });
                  setTheme(value);
                }}
                className={cn(active && "border-brand bg-brand-soft text-brand")}
              >
                <Icon size={13} /> {t(label)}
              </Button>
            );
          })}
        </Row>
      </Card>

      <Card title={t("言語")} icon={<Zap size={13} />}>
        <Row className="gap-1.5">
          {(["ja", "en"] as const).map((language) => {
            const active = (data.language ?? "ja") === language;
            return (
              <Button
                key={language}
                size="sm"
                aria-pressed={active}
                onClick={() => {
                  patch({ language });
                  setLocale(language);
                }}
                className={cn(active && "border-brand bg-brand-soft text-brand")}
              >
                {language === "ja" ? "日本語" : "English"}
              </Button>
            );
          })}
        </Row>
      </Card>
    </Section>
  );
}
