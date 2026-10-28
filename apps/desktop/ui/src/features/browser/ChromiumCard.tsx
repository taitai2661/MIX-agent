import { Download, RefreshCw, Trash2, X } from "lucide-react";
import { t } from "@/lib/i18n";
import { Card, Row } from "@/components/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import type { BrowserStatus } from "@/lib/types";

/** Kept as literal tables so the i18n coverage test can see every key. */
const SOURCE_KEYS: Record<BrowserStatus["source"], string> = {
  installed: "installed",
  bundled: "bundled",
  system: "system",
  override: "override",
  none: "none",
};
const STAGE_KEYS: Record<string, string> = {
  preparing: "preparing",
  downloading: "downloading",
  verifying: "verifying",
  extracting: "extracting",
  launching: "launching",
  done: "done",
};

interface Props {
  status: BrowserStatus | null;
  loading: boolean;
  installActive: boolean;
  installBusy: boolean;
  onInstall: (force: boolean) => void;
  onCancel: () => void;
  onUninstall: () => void;
}

/**
 * Managed Chromium lifecycle: version, source and install progress.
 *
 * Downloading is the only stage with a real percentage. Verifying and
 * extracting used to render a bar pinned at 99%, which read as a hang; they
 * are rendered as an indeterminate bar instead.
 */
export function ChromiumCard({
  status,
  loading,
  installActive,
  installBusy,
  onInstall,
  onCancel,
  onUninstall,
}: Props) {
  const { confirm, dialog } = useConfirm();
  const install = status?.install;
  const determinate = install?.status === "downloading" && install.bytes_total;
  const progress = install?.progress ?? 0;

  const remove = async () => {
    const ok = await confirm({
      title: t("Chromiumを削除しますか？"),
      description: t(
        "管理対象のChromiumだけを削除します。システムのブラウザは残ります。",
      ),
      confirmLabel: t("削除"),
      tone: "danger",
    });
    if (ok) onUninstall();
  };

  return (
    <Card
      title={t("Chromium")}
      icon={<Download size={13} />}
      action={
        installActive ? (
          <Button variant="ghost" size="sm" onClick={onCancel}>
            <X size={12} /> {t("キャンセル")}
          </Button>
        ) : undefined
      }
    >
      <Row className="gap-1.5">
        <Badge
          size="sm"
          tone={
            install?.status === "ready"
              ? "success"
              : install?.status === "failed"
                ? "danger"
                : "neutral"
          }
        >
          {install?.status === "ready"
            ? t("インストール済み")
            : install?.status === "failed"
              ? t("失敗")
              : install?.status === "cancelled"
                ? t("キャンセル済み")
                : installActive
                  ? t("インストール中")
                  : t("未インストール")}
        </Badge>
        {status?.installed_version && (
          <Badge size="sm" className="font-mono">
            {status.installed_version === "unknown"
              ? t("バージョン不明")
              : `v${status.installed_version}`}
          </Badge>
        )}
        {status?.source && status.source !== "none" && (
          <Badge size="sm">{t(SOURCE_KEYS[status.source])}</Badge>
        )}
        {status?.update_available && status.installed_version && (
          <Badge size="sm" tone="warning">
            {t("更新あり")}
          </Badge>
        )}
      </Row>

      {installActive ? (
        <div className="flex flex-col gap-1.5">
          <div
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={determinate ? progress : undefined}
            className="h-1.5 w-full overflow-hidden rounded-full bg-inset"
          >
            <div
              className={
                determinate
                  ? "h-full rounded-full bg-brand transition-[width] duration-150"
                  : "h-full w-full animate-pulse rounded-full bg-brand"
              }
              style={determinate ? { width: `${progress}%` } : undefined}
            />
          </div>
          <p className="m-0 text-xs text-muted">
            {t(STAGE_KEYS[install?.stage ?? "downloading"] ?? "downloading")}
            {determinate
              ? ` · ${Math.round((install?.bytes_done ?? 0) / (1024 * 1024))} / ${Math.round(
                  (install?.bytes_total ?? 0) / (1024 * 1024),
                )} MB`
              : ""}
          </p>
        </div>
      ) : install?.status === "failed" ? (
        <p className="m-0 text-xs text-danger">
          {install.failure ?? t("インストールに失敗しました")}
        </p>
      ) : status?.installed_version ? (
        <p className="m-0 text-xs text-muted">
          {t(
            "Chrome for Testingをアプリ管理下にインストールしています。更新は次回のアプリ更新時に確認できます。",
          )}
        </p>
      ) : (
        <p className="m-0 text-xs text-muted">
          {t(
            "ブラウザ機能を使うにはChromiumをインストールしてください。約200MBをダウンロードします。",
          )}
        </p>
      )}

      <Row className="gap-1.5">
        {!installActive &&
          install?.status !== "failed" &&
          install?.status !== "cancelled" &&
          (!status?.installed_version || status.update_available) && (
            <Button
              variant="primary"
              size="sm"
              disabled={installBusy || loading}
              onClick={() => onInstall(Boolean(status?.installed_version))}
            >
              <Download size={12} />
              {status?.installed_version ? t("更新する") : t("インストール")}
            </Button>
          )}
        {(install?.status === "failed" || install?.status === "cancelled") && (
          <Button
            size="sm"
            disabled={installBusy}
            onClick={() => onInstall(false)}
          >
            <RefreshCw size={12} /> {t("再試行")}
          </Button>
        )}
        {status?.source === "installed" && !installActive && (
          <Button
            variant="ghost-danger"
            size="sm"
            onClick={() => void remove()}
          >
            <Trash2 size={12} /> {t("削除")}
          </Button>
        )}
      </Row>
      {dialog}
    </Card>
  );
}
