import { Monitor, Moon, Palette, Sun, Zap } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import type { Theme } from "@/lib/theme";
import { useApplySettings, useSettingsForm } from "@/lib/settings";
import { Card, Row, Section } from "@/components/layout";
import { Button } from "@/components/ui/button";

const THEMES: Array<{ value: Theme; label: string; Icon: typeof Sun }> = [
  { value: "light", label: "ライト", Icon: Sun },
  { value: "dark", label: "ダーク", Icon: Moon },
  { value: "system", label: "システムに従う", Icon: Monitor },
];

export function AppearanceSettings() {
  const { data } = useSettingsForm();
  const apply = useApplySettings();
  const setTheme = useUi((s) => s.setTheme);
  const setLocale = useUi((s) => s.setLocale);
  const locale = useUi((s) => s.locale);

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
                  setTheme(value);
                  void apply({ theme: value });
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
            const active = locale === language;
            return (
              <Button
                key={language}
                size="sm"
                aria-pressed={active}
                onClick={() => {
                  setLocale(language);
                  void apply({ language });
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
