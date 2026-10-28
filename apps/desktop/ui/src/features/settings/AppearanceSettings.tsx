import {
  Minus,
  Monitor,
  Moon,
  Palette,
  Plus,
  RotateCcw,
  Sun,
  Zap,
} from "lucide-react";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import type { Theme } from "@/lib/theme";
import { useApplySettings, useSettingsForm } from "@/lib/settings";
import { Card, Row, Section } from "@/components/layout";
import { ToggleRow } from "@/components/common";
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
  const zoom = useUi((s) => s.zoom);
  const setZoom = useUi((s) => s.setZoom);
  const codeWrap = useUi((s) => s.codeWrap);
  const setCodeWrap = useUi((s) => s.setCodeWrap);
  const reduceMotion = useUi((s) => s.reduceMotion);
  const setReduceMotion = useUi((s) => s.setReduceMotion);

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
                selected={active}
                onClick={() => {
                  setTheme(value);
                  void apply({ theme: value });
                }}
              >
                <Icon size={12} /> {t(label)}
              </Button>
            );
          })}
        </Row>
      </Card>

      <Card title={t("表示")} icon={<Monitor size={13} />}>
        <div className="flex flex-col gap-1.5">
          <span className="text-xs font-medium text-muted select-none">
            {t("表示倍率")}
          </span>
          <Row className="gap-1.5">
            <Button
              size="sm"
              onClick={() => setZoom(zoom - 0.1)}
              disabled={zoom <= 0.8}
              aria-label={t("縮小")}
              title={t("縮小")}
            >
              <Minus size={12} />
            </Button>
            <span className="min-w-14 text-center font-mono text-xs tabular-nums text-fg">
              {Math.round(zoom * 100)}%
            </span>
            <Button
              size="sm"
              onClick={() => setZoom(zoom + 0.1)}
              disabled={zoom >= 1.5}
              aria-label={t("拡大")}
              title={t("拡大")}
            >
              <Plus size={12} />
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setZoom(1)}
              disabled={zoom === 1}
            >
              <RotateCcw size={12} /> {t("リセット")}
            </Button>
          </Row>
          <span className="text-xs text-muted">
            {t(
              "アプリ全体を拡大・縮小します。キーボードショートカットでも変更できます（? で一覧）。",
            )}
          </span>
        </div>
        <ToggleRow
          label={t("コードを折り返す")}
          hint={t(
            "長いコードブロックを横スクロールせず、画面幅で折り返して表示します。",
          )}
          checked={codeWrap}
          onChange={setCodeWrap}
        />
        <ToggleRow
          label={t("アニメーションを減らす")}
          hint={t(
            "画面遷移や点滅を抑えます。OSの「視差効果を減らす」設定に加えて適用されます。",
          )}
          checked={reduceMotion}
          onChange={setReduceMotion}
        />
      </Card>

      <Card title={t("言語")} icon={<Zap size={13} />}>
        <Row className="gap-1.5">
          {(["ja", "en"] as const).map((language) => {
            const active = locale === language;
            return (
              <Button
                key={language}
                size="sm"
                selected={active}
                onClick={() => {
                  setLocale(language);
                  void apply({ language });
                }}
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
