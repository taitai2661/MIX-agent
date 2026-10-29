import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { Download, RefreshCw, RotateCw } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { Card, Section } from "@/components/layout";
import { ToggleRow } from "@/components/common";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import { useSettingsForm } from "@/lib/settings";
import {
  isLinuxPlatform,
  isMacPlatform,
  isWindowsPlatform,
} from "@/lib/platform";
import { UPDATE_QUERY_KEY, useUpdateCheck } from "@/lib/updates";
import type { UpdateProgress } from "@/lib/types";

const RELEASES_URL = "https://github.com/taitai2661/MIX-agent/releases";

/**
 * In-app update flow.
 *
 * The Rust side owns check/download/install (`commands::update`); this pane
 * only renders state and streams `update://progress`. On Windows the installer
 * exits the process by itself; on macOS/Linux the user is offered a restart so
 * the freshly installed build is the one that runs.
 */
export function UpdateSettings() {
  const { data, patch, loaded } = useSettingsForm();
  const queryClient = useQueryClient();
  const [version, setVersion] = useState<string | null>(null);
  // Shares `UPDATE_QUERY_KEY` with the header badge, so opening this pane does
  // not fire a second request and an install clears both views at once.
  const checkQuery = useUpdateCheck(loaded && data.auto_check_updates);
  const update = checkQuery.data ?? null;
  const { confirm, dialog } = useConfirm();
  const [installing, setInstalling] = useState(false);
  const [installed, setInstalled] = useState<string | null>(null);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .appVersion()
      .then(setVersion)
      .catch(() => setVersion(null));
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen<UpdateProgress>("update://progress", (event) => {
      setProgress(event.payload);
    })
      .then((fn) => {
        // The effect may have been torn down before `listen` resolved.
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // Older builds without the command simply never emit progress.
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const requestInstall = async () => {
    // The button sits one click away from an alert and used to start an
    // installer immediately, even though the hint beside it promised a
    // confirmation dialog. Installing quits/restarts the app on Windows.
    const ok = await confirm({
      title: tf("v{version} をインストールしますか？", {
        version: update?.version ?? "",
      }),
      description: t(
        "ダウンロードとインストールを開始します。インストール後はアプリの再起動が必要です。作業中の実行は完了してからにしてください。",
      ),
      confirmLabel: t("ダウンロードしてインストール"),
    });
    if (ok) void install();
  };

  const install = async () => {
    setInstalling(true);
    setError(null);
    setProgress(null);
    try {
      const info = await api.installUpdate();
      setInstalled(info.version);
      queryClient.setQueryData(UPDATE_QUERY_KEY, null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setInstalling(false);
    }
  };

  const percent =
    progress && progress.total && progress.total > 0
      ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100))
      : null;
  const queryError = checkQuery.isError ? errorMessage(checkQuery.error) : null;
  const upToDate =
    checkQuery.isFetched && !checkQuery.isError && checkQuery.data === null;

  return (
    <Section>
      <Card title={t("アプリのアップデート")} icon={<RefreshCw size={13} />}>
        <div className="flex flex-col gap-3 py-1">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="m-0 text-sm text-fg">
              {version
                ? tf("現在のバージョン: {version}", { version })
                : t("現在のバージョンを取得できませんでした")}
            </p>
            <Button
              size="sm"
              disabled={checkQuery.isFetching || installing}
              onClick={() => void checkQuery.refetch()}
            >
              <RefreshCw
                size={12}
                className={checkQuery.isFetching ? "animate-spin" : ""}
              />
              {t("更新を確認")}
            </Button>
          </div>

          {error && <Alert tone="danger">{error}</Alert>}
          {!error && queryError && <Alert tone="danger">{queryError}</Alert>}

          {installing && (
            <div className="flex flex-col gap-1.5">
              <div
                className="h-1.5 w-full overflow-hidden rounded-full bg-inset"
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={percent ?? undefined}
              >
                <div
                  className="h-full rounded-full bg-brand transition-[width] duration-200"
                  style={{ width: `${percent ?? 5}%` }}
                />
              </div>
              <p className="m-0 text-xs text-muted">
                {percent === null
                  ? t("ダウンロード中…")
                  : tf("ダウンロード中… {percent}%", { percent })}
              </p>
            </div>
          )}

          {installed && !installing && (
            <Alert
              tone="success"
              action={
                <Button size="sm" onClick={() => void api.relaunchApp()}>
                  <RotateCw size={12} /> {t("再起動")}
                </Button>
              }
            >
              {tf(
                "v{version} をインストールしました。再起動すると適用されます。",
                { version: installed },
              )}
            </Alert>
          )}

          {update && !installing && !installed && (
            <Alert
              tone={update.installable ? "info" : "warning"}
              action={
                update.installable ? (
                  <Button
                    size="sm"
                    variant="primary"
                    onClick={() => void requestInstall()}
                  >
                    <Download size={12} /> {t("ダウンロードしてインストール")}
                  </Button>
                ) : undefined
              }
            >
              <div className="flex flex-col gap-1">
                <span className="font-medium">
                  {tf("v{version} が利用可能です", { version: update.version })}
                  {update.date && (
                    <span className="ms-2 font-normal text-muted">
                      {update.date}
                    </span>
                  )}
                </span>
                {update.notes && (
                  <span className="whitespace-pre-wrap text-xs">
                    {update.notes}
                  </span>
                )}
                {!update.installable && (
                  <span className="text-xs">
                    {t(
                      "このビルドはアプリ内更新に対応していません。GitHub Releasesから手動で更新してください。",
                    )}
                  </span>
                )}
              </div>
            </Alert>
          )}

          {upToDate && !installed && !installing && !error && !queryError && (
            <Alert tone="success">{t("最新バージョンです。")}</Alert>
          )}

          <div className="border-t border-line pt-3">
            <ToggleRow
              label={t("起動時に更新を確認")}
              hint={t(
                "起動時にGitHub Releasesのマニフェストを確認します。ダウンロードとインストールは署名を検証した上で、確認ダイアログから実行します。",
              )}
              checked={data.auto_check_updates}
              onChange={(auto_check_updates) => patch({ auto_check_updates })}
            />
          </div>

          {isLinuxPlatform() && (
            <p className="m-0 text-xs text-muted">
              {t(
                "Linuxのdebパッケージはアプリ内更新の対象外です。debで導入した場合はGitHub Releasesから手動で更新してください（AppImageは自動更新できます）。",
              )}
            </p>
          )}
          <a
            href={RELEASES_URL}
            target="_blank"
            rel="noreferrer"
            className="inline-flex w-fit items-center gap-2 text-sm font-medium text-brand underline underline-offset-4"
          >
            <Download size={14} aria-hidden="true" />
            {t("GitHub Releasesを開く")}
          </a>
          {isMacPlatform() && (
            <p className="m-0 text-xs text-muted">
              {t(
                "macOSアプリはApple Developer ID署名・公証なし（ad-hoc署名のみ）です。初回起動時にGatekeeperの確認が必要です。",
              )}
            </p>
          )}
          {isWindowsPlatform() && (
            <p className="m-0 text-xs text-muted">
              {t(
                "Windowsインストーラーは未署名です。SmartScreenの警告が出る場合は「詳細情報」→「実行」を選択してください。",
              )}
            </p>
          )}
        </div>
      </Card>
      {dialog}
    </Section>
  );
}
