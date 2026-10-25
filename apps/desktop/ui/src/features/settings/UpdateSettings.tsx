import { Download } from "lucide-react";
import { t } from "@/lib/i18n";
import { Card, Section } from "@/components/layout";
import { ToggleRow } from "@/components/common";
import { useSettingsForm } from "@/lib/settings";

const RELEASES_URL = "https://github.com/taitai2661/MIX-agent/releases";

export function UpdateSettings() {
  const { data, patch } = useSettingsForm();

  return (
    <Section>
      <Card title={t("手動アップデート")} icon={<Download size={13} />}>
        <div className="flex flex-col gap-3 py-1">
          <p className="text-sm text-fg">
            {t(
              "v0.5.0では自動更新とアプリ内の更新確認を無効にしています。最新版はGitHub ReleasesからMac用DMGを手動でダウンロードしてください。",
            )}
          </p>
          <a
            href={RELEASES_URL}
            target="_blank"
            rel="noreferrer"
            className="inline-flex w-fit items-center gap-2 text-sm font-medium text-brand underline underline-offset-4"
          >
            <Download size={14} aria-hidden="true" />
            {t("GitHub Releasesを開く")}
          </a>
          <p className="text-xs text-muted">
            {t("ダウンロードしたDMGのSHA256SUMSチェックサムを確認してください。")}
          </p>
          <p className="text-xs text-muted">
            {t(
              "macOSアプリはAppleコード署名・公証なしです。Gatekeeperが警告または起動を制限する場合があります。",
            )}
          </p>
          <div className="border-t border-line pt-3">
            <ToggleRow
              label={t("匿名の利用統計を共有 (将来)")}
              hint={t("アプリの改善のため匿名の診断情報を共有します")}
              checked={data.telemetry_consent}
              onChange={(telemetry_consent) => patch({ telemetry_consent })}
            />
          </div>
        </div>
      </Card>
    </Section>
  );
}
