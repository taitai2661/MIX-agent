import { useEffect, useState } from "react";
import { Download, RefreshCw } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { Card, Section } from "@/components/layout";
import { ToggleRow } from "@/components/common";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { useSettingsForm } from "@/lib/settings";
import {
  isLinuxPlatform,
  isMacPlatform,
  isWindowsPlatform,
} from "@/lib/platform";
import {
  clearUpdateError,
  downloadPendingUpdate,
  useUpdateCheck,
  useUpdateStore,
} from "@/lib/updates";

const RELEASES_URL = "https://github.com/taitai2661/MIX-agent/releases";

/**
 * In-app update flow.
 *
 * The Rust side owns check/download and installs the staged payload when the
 * app quits (`commands::update`); this pane only renders state and starts the
 * manual download. There is deliberately no "restart now" button: the update
 * applies on the next normal launch.
 */
export function UpdateSettings() {
  const { data, patch, loaded } = useSettingsForm();
  const [version, setVersion] = useState<string | null>(null);
  const [restarting, setRestarting] = useState(false);
  const [restartError, setRestartError] = useState<string | null>(null);
  // Shares `UPDATE_QUERY_KEY` with the header badge, so opening this pane does
  // not fire a second request.
  const checkQuery = useUpdateCheck(loaded && data.auto_check_updates);
  const update = checkQuery.data ?? null;
  const phase = useUpdateStore((state) => state.phase);
  const pending = useUpdateStore((state) => state.pending);
  const progress = useUpdateStore((state) => state.progress);
  const downloadError = useUpdateStore((state) => state.error);
  const installing = phase === "downloading";

  useEffect(() => {
    api
      .appVersion()
      .then(setVersion)
      .catch(() => setVersion(null));
  }, []);

  const verifying = progress?.stage === "verifying";
  let percent: number | null = null;
  if (progress) {
    if (verifying) percent = 100;
    else if (progress.total && progress.total > 0) {
      percent = Math.min(
        100,
        Math.round((progress.downloaded / progress.total) * 100),
      );
    }
  }
  // `check_for_update` already reports the running version. Use it when the
  // separate `app_version` call failed (or is still in flight) instead of
  // telling the user the version is unknown.
  const displayVersion =
    version ?? update?.current_version ?? pending?.current_version ?? null;
  const queryError = checkQuery.isError ? errorMessage(checkQuery.error) : null;
  const upToDate =
    checkQuery.isFetched &&
    !checkQuery.isError &&
    checkQuery.data === null &&
    !pending &&
    !installing;

  return (
    <Section>
      <Card title={t("アプリのアップデート")} icon={<RefreshCw size={13} />}>
        <div className="flex flex-col gap-3 py-1">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="m-0 text-sm text-fg">
              {displayVersion
                ? tf("現在のバージョン: {version}", { version: displayVersion })
                : t("現在のバージョンを取得できませんでした")}
            </p>
            <Button
              size="sm"
              disabled={checkQuery.isFetching || installing}
              onClick={() => {
                clearUpdateError();
                void checkQuery.refetch();
              }}
            >
              <RefreshCw
                size={12}
                className={checkQuery.isFetching ? "animate-spin" : ""}
              />
              {t("更新を確認")}
            </Button>
          </div>

          {downloadError && <Alert tone="danger">{downloadError}</Alert>}
          {!downloadError && queryError && (
            <Alert tone="danger">{queryError}</Alert>
          )}

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
                {verifying
                  ? t("署名を検証しています…")
                  : percent === null
                    ? t("ダウンロード中…")
                    : tf("ダウンロード中… {percent}%", { percent })}
              </p>
            </div>
          )}

          {pending && !installing && (
            <Alert tone="info">
              <div className="flex flex-col gap-1">
                <span className="font-medium">
                  {tf(
                    "v{version} をダウンロードしました。次回 MIX-agent を終了（Quit）すると適用されます。",
                    { version: pending.version },
                  )}
                </span>
                {pending.date && (
                  <span className="text-xs text-muted">{pending.date}</span>
                )}
                {pending.notes && (
                  <span className="whitespace-pre-wrap text-xs">
                    {pending.notes}
                  </span>
                )}
                <div className="mt-1">
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={restarting}
                    onClick={async () => {
                      setRestarting(true);
                      setRestartError(null);
                      try {
                        await api.installUpdateNow();
                      } catch (err) {
                        setRestartError(errorMessage(err));
                        setRestarting(false);
                      }
                    }}
                  >
                    <RefreshCw size={12} />
                    {restarting ? t("再起動しています…") : t("再起動して更新")}
                  </Button>
                </div>
                {restartError && <Alert tone="danger">{restartError}</Alert>}
              </div>
            </Alert>
          )}

          {update && !installing && !pending && (
            <Alert
              tone={update.installable ? "info" : "warning"}
              action={
                update.installable ? (
                  <Button
                    size="sm"
                    variant="primary"
                    disabled={checkQuery.isFetching}
                    onClick={() => void downloadPendingUpdate()}
                  >
                    <Download size={12} /> {t("ダウンロード")}
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

          {upToDate && (
            <Alert tone="success">{t("最新バージョンです。")}</Alert>
          )}

          <div className="border-t border-line pt-3">
            <ToggleRow
              label={t("起動時に更新を確認")}
              hint={t("起動時にGitHub Releasesのマニフェストを確認します。")}
              checked={data.auto_check_updates}
              onChange={(auto_check_updates) => patch({ auto_check_updates })}
            />
            <ToggleRow
              className="mt-3"
              label={t("更新を自動でダウンロード")}
              hint={t(
                "起動時に新しいバージョンを見つけたら自動でダウンロードします。適用はメニュー／トレイの Quit で終了したときに行われ、再起動は手動です。",
              )}
              checked={data.auto_download_updates}
              disabled={!data.auto_check_updates}
              onChange={(auto_download_updates) =>
                patch({ auto_download_updates })
              }
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
    </Section>
  );
}
