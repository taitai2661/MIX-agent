import { useEffect } from "react";
import {
  AlertCircle,
  CheckCircle2,
  Download,
  Info,
  RefreshCw,
  Sparkles,
} from "lucide-react";
import { t } from "@/lib/i18n";
import { Card, Section } from "@/components/layout";
import { ToggleRow } from "@/components/common";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Spinner } from "@/components/ui/spinner";
import { Markdown } from "@/components/Markdown";
import { useSettingsForm } from "@/lib/settings";
import { useUpdater } from "@/lib/updater";

export function UpdateSettings() {
  const { data, patch } = useSettingsForm();
  const {
    update,
    currentVersion,
    isChecking,
    isInstalling,
    downloadProgress,
    error,
    lastChecked,
    isUpToDate,
    check,
    install,
    init,
  } = useUpdater();

  useEffect(() => {
    return init();
  }, [init]);

  return (
    <Section>
      {/* Current Version & Update Check */}
      <Card title={t("アップデート状態")} icon={<RefreshCw size={13} />}>
        <div className="flex flex-col gap-4 py-1">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <div className="flex items-center gap-2">
                <span className="text-sm font-semibold text-fg">
                  MIX-agent Desktop
                </span>
                <Badge tone="neutral" size="sm">
                  v{currentVersion}
                </Badge>
              </div>
              <p className="mt-0.5 text-xs text-muted">
                {lastChecked
                  ? `${t("最終確認")}: ${lastChecked.toLocaleTimeString()}`
                  : t("GitHub Releases 経由で最新リリースを確認します")}
              </p>
            </div>

            <Button
              variant="default"
              size="sm"
              disabled={isChecking || isInstalling}
              onClick={() => void check()}
            >
              {isChecking ? (
                <span className="flex items-center gap-1.5">
                  <Spinner size={12} />
                  {t("確認中...")}
                </span>
              ) : (
                <span className="flex items-center gap-1.5">
                  <RefreshCw size={12} />
                  {t("今すぐ更新を確認")}
                </span>
              )}
            </Button>
          </div>

          {/* Status feedback */}
          {isUpToDate && !update && !isChecking && (
            <div className="flex items-center gap-2 rounded-md bg-success-soft/30 px-3 py-2 text-xs text-fg">
              <CheckCircle2 size={14} className="shrink-0 text-success" />
              <span>{t("お使いのバージョンは最新です")}</span>
            </div>
          )}

          {error && !isChecking && (
            <div className="flex items-start gap-2 rounded-md bg-danger-soft/40 px-3 py-2 text-xs text-fg">
              <AlertCircle size={14} className="mt-0.5 shrink-0 text-danger" />
              <div className="min-w-0 flex-1">
                <span className="font-medium text-danger">
                  {t("更新の確認に失敗しました")}
                </span>
                <p className="mt-0.5 text-muted break-all">{error}</p>
              </div>
            </div>
          )}
        </div>
      </Card>

      {/* Available Update Details */}
      {update && (
        <Card
          title={t("利用可能なアップデート")}
          icon={<Sparkles size={13} className="text-brand" />}
        >
          <div className="flex flex-col gap-3 py-1">
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-brand/20 bg-brand-soft/40 p-3">
              <div>
                <div className="flex items-center gap-2">
                  <Badge tone="brand" size="md">
                    v{update.version}
                  </Badge>
                  {update.date && (
                    <span className="text-xs text-muted">
                      {new Date(update.date).toLocaleDateString()}
                    </span>
                  )}
                </div>
                <p className="mt-1 text-xs text-fg">
                  v{update.current_version} → v{update.version}
                </p>
              </div>

              <Button
                variant="primary"
                size="sm"
                disabled={isInstalling}
                onClick={() => void install()}
              >
                {isInstalling ? (
                  <span className="flex items-center gap-1.5">
                    <Spinner size={12} />
                    {downloadProgress?.percent != null
                      ? `${Math.round(downloadProgress.percent)}%`
                      : t("ダウンロード中...")}
                  </span>
                ) : (
                  <span className="flex items-center gap-1.5">
                    <Download size={12} />
                    {t("ダウンロードして再起動")}
                  </span>
                )}
              </Button>
            </div>

            {/* Download Progress Bar */}
            {isInstalling && (
              <div className="rounded-md border border-line bg-surface p-3">
                <div className="flex items-center justify-between text-xs text-muted">
                  <span>{t("アップデートをダウンロード中...")}</span>
                  {downloadProgress?.percent != null && (
                    <span className="font-mono font-medium text-fg">
                      {Math.round(downloadProgress.percent)}%
                    </span>
                  )}
                </div>
                <div className="mt-2 h-2 w-full overflow-hidden rounded-full bg-line">
                  <div
                    className="h-full bg-brand transition-all duration-150"
                    style={{
                      width: `${downloadProgress?.percent ?? 0}%`,
                    }}
                  />
                </div>
                {downloadProgress?.total_bytes && (
                  <p className="mt-1.5 text-right font-mono text-[10px] text-muted">
                    {(downloadProgress.downloaded_bytes / 1024 / 1024).toFixed(
                      1,
                    )}{" "}
                    / {(downloadProgress.total_bytes / 1024 / 1024).toFixed(1)}{" "}
                    MB
                  </p>
                )}
              </div>
            )}

            {/* Release Notes */}
            {update.notes && (
              <div className="flex flex-col gap-1.5">
                <span className="text-xs font-medium text-fg">
                  {t("リリースノート")}
                </span>
                <div className="max-h-48 overflow-y-auto rounded-md border border-line bg-surface p-3 text-xs">
                  <Markdown>{update.notes}</Markdown>
                </div>
              </div>
            )}
          </div>
        </Card>
      )}

      {/* Auto-update Settings */}
      <Card title={t("アップデート設定")} icon={<Info size={13} />}>
        <ToggleRow
          label={t("自動更新を有効化")}
          hint={t("起動時にバックグラウンドで新しいバージョンを確認します")}
          checked={data.auto_update}
          onChange={(auto_update) => patch({ auto_update })}
        />
        <ToggleRow
          label={t("匿名の利用統計を共有 (将来)")}
          hint={t("アプリの改善のため匿名の診断情報を共有します")}
          checked={data.telemetry_consent}
          onChange={(telemetry_consent) => patch({ telemetry_consent })}
        />
      </Card>
    </Section>
  );
}
