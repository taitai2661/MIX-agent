import { useCallback, useEffect, useState } from "react";
import { ChevronRight, ExternalLink, Globe, RefreshCw } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { Card, Row, Section } from "@/components/layout";
import { EmptyState, payloadBlock } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";
import { Select } from "@/components/ui/input";
import { BrowserActionForm } from "./BrowserActionForm";
import { ChromiumCard } from "./ChromiumCard";
import { LiveViewCard } from "./LiveViewCard";
import { makeLogEntry, type LogEntry } from "./browserLog";
import { useBrowserStatus } from "./useBrowserStatus";
import { useLiveFrame } from "./useLiveFrame";

/** Lazy screenshot preview for one artifact id. */
function ArtifactThumb({ id }: { id: string }) {
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let cancelled = false;
    api
      .browserArtifact(id)
      .then((url) => {
        if (!cancelled) setSrc(url);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [id]);
  if (failed) {
    return (
      <Badge size="sm" tone="danger">
        {t("プレビューを読み込めません")}
      </Badge>
    );
  }
  if (!src) return <Spinner size={12} label={t("読み込み中…")} />;
  return (
    <a
      href={src}
      download={`browser-${id.slice(0, 8)}.png`}
      className="mt-1.5 block w-fit"
      title={t("スクリーンショットを保存")}
    >
      <img
        src={src}
        alt={t("ブラウザのスクリーンショット")}
        className="max-h-40 rounded-md border border-line object-contain"
      />
    </a>
  );
}

export function BrowserPane() {
  // Default to the active conversation's workspace so a manual sign-in lands in
  // the profile the agent will later browse with.
  const activeWorkspace = useUi((s) => s.workspaceId);
  const [workspaceId, setWorkspaceId] = useState<string>(
    () => activeWorkspace ?? crypto.randomUUID(),
  );
  const [log, setLog] = useState<LogEntry[]>([]);
  const { confirm, dialog } = useConfirm();

  // Keep the manual-sign-in profile on the active conversation's workspace.
  // The initial state captures it once; without this the pane goes stale
  // after switching threads.
  useEffect(() => {
    if (activeWorkspace) setWorkspaceId(activeWorkspace);
  }, [activeWorkspace]);

  const reportError = useCallback((e: unknown) => {
    setLog((l) =>
      [
        makeLogEntry({
          action: "info",
          ok: false,
          summary: errorMessage(e),
        }),
        ...l,
      ].slice(0, 50),
    );
  }, []);

  const onResult = useCallback((entry: Omit<LogEntry, "id">) => {
    setLog((l) => [makeLogEntry(entry), ...l].slice(0, 50));
  }, []);

  const {
    status,
    profiles,
    loading,
    installBusy,
    installActive,
    refresh,
    startInstall,
    cancelInstall,
    removeChromium,
  } = useBrowserStatus(reportError);
  const live = useLiveFrame(workspaceId);

  const knownProfile = profiles.some((p) => p.workspace_id === workspaceId);

  return (
    <Section>
      <Card
        title={t("バックエンド")}
        icon={<Globe size={13} />}
        action={
          <Button
            variant="ghost"
            size="sm"
            onClick={() => void refresh()}
            title={t("状態を更新")}
          >
            <RefreshCw size={12} /> {t("更新")}
          </Button>
        }
      >
        <Row className="gap-1.5">
          <Badge size="sm" className="font-mono">
            {status?.backend ?? (loading ? t("読み込み中…") : "—")}
          </Badge>
          <Badge size="sm">
            {status?.headless ? t("ヘッドレス") : t("ヘッドあり")}
          </Badge>
          <Badge size="sm">
            {status?.profiles ?? 0} {t("プロファイル")}
          </Badge>
          <Badge size="sm">
            {status?.artifacts ?? 0} {t("成果物")}
          </Badge>
          {status?.allow_local && (
            <Badge size="sm" tone="warning">
              {t("ローカルアクセス許可")}
            </Badge>
          )}
        </Row>
        {status?.executable ? (
          <code className="break-all text-xs text-faint">
            {status.executable}
          </code>
        ) : loading ? (
          <span className="inline-flex items-center gap-1.5 text-xs text-muted">
            <Spinner size={12} /> {t("読み込み中…")}
          </span>
        ) : (
          <div className="flex flex-col gap-2">
            <p className="m-0 text-xs text-muted">
              {t(
                "Chromiumが見つかりません。上の「インストール」から導入するか、システムのChromiumを用意してください。",
              )}
            </p>
            <Button
              size="sm"
              onClick={() =>
                void api.openExternal(
                  "https://github.com/taitai2661/MIX-agent/releases",
                )
              }
            >
              <ExternalLink size={12} /> {t("Releases を開く")}
            </Button>
          </div>
        )}
      </Card>

      <ChromiumCard
        status={status}
        loading={loading}
        installActive={installActive}
        installBusy={installBusy}
        onInstall={(force) => void startInstall(force)}
        onCancel={() => void cancelInstall()}
        onUninstall={() => void removeChromium()}
      />

      <LiveViewCard live={live} />

      <Card title={t("ワークスペースのプロファイル")}>
        <Row className="gap-2">
          <Select
            value={knownProfile ? workspaceId : "new"}
            onChange={(e) =>
              setWorkspaceId(
                e.target.value === "new" ? crypto.randomUUID() : e.target.value,
              )
            }
          >
            {!knownProfile && (
              <option value="new">{t("新規プロファイル")}</option>
            )}
            {profiles.map((p) => (
              <option key={p.workspace_id} value={p.workspace_id}>
                {p.display_name} {p.provisioned ? "" : t("(未作成)")}
              </option>
            ))}
          </Select>
          <code className="min-w-0 flex-1 truncate font-mono text-xs text-faint">
            {workspaceId}
          </code>
          <Button
            onClick={async () => {
              // Wiping a profile's cookies, storage and cache is not reversible,
              // so it is a `danger` confirmation like every other destructive
              // action rather than the native alert it used to be.
              const ok = await confirm({
                title: t("プロファイルをリセットしますか？"),
                description: t("Cookie・localStorage・キャッシュが消えます。"),
                confirmLabel: t("リセット"),
                tone: "danger",
              });
              if (!ok) return;
              try {
                await api.browserResetProfile(workspaceId);
                // Stay on the same workspace: switching to a random UUID made
                // the next manual sign-in land in a profile the conversation
                // would never use.
                live.clearFrame();
                await refresh();
              } catch (e) {
                reportError(e);
              }
            }}
          >
            {t("リセット")}
          </Button>
        </Row>
      </Card>

      <BrowserActionForm workspaceId={workspaceId} onResult={onResult} />

      <Card title={t("アクティビティ")}>
        {log.length === 0 ? (
          <EmptyState title={t("まだ操作がありません。")} />
        ) : (
          <ul className="flex flex-col">
            {log.map((e) => (
              <li
                key={e.id}
                className="border-t border-line py-2 first:border-0 first:pt-0"
              >
                <Row className="gap-2">
                  <Badge size="sm" className="font-mono">
                    {e.action}
                  </Badge>
                  <span
                    className={cn(
                      "text-xs",
                      e.ok ? "text-success" : "text-danger",
                    )}
                  >
                    {e.ok ? t("成功") : t("失敗")}: {e.summary}
                  </span>
                  {e.artifactId && (
                    <Badge size="sm" className="font-mono">
                      artifact {e.artifactId.slice(0, 8)}
                    </Badge>
                  )}
                </Row>
                {e.artifactId && <ArtifactThumb id={e.artifactId} />}
                {e.detail && (
                  <details className="group">
                    <summary className="mt-1 flex cursor-pointer list-none items-center gap-1 text-xs text-muted transition-colors hover:text-fg">
                      <ChevronRight
                        size={12}
                        aria-hidden="true"
                        className="shrink-0 transition-transform group-open:rotate-90"
                      />
                      {t("出力")}
                    </summary>
                    <pre
                      className={`${payloadBlock} mt-1.5 max-h-64 break-words`}
                    >
                      {e.detail}
                    </pre>
                  </details>
                )}
              </li>
            ))}
          </ul>
        )}
      </Card>

      {profiles.length > 0 && (
        <Card title={t("既知のプロファイル")}>
          <ul className="flex flex-col gap-1 text-xs">
            {profiles.map((p) => (
              <li key={p.workspace_id} className="flex items-center gap-2">
                <span className="text-muted">{p.display_name}</span>
                <code className="font-mono text-xs text-faint">
                  {p.workspace_id.slice(0, 8)}
                </code>
                <Badge tone={p.provisioned ? "success" : "neutral"} size="sm">
                  {p.provisioned ? t("準備完了") : t("空")}
                </Badge>
              </li>
            ))}
          </ul>
          <p className="m-0 text-xs text-muted">
            {t(
              "各ワークスペースは独自のCookie・localStorage・キャッシュを持ち、エージェントのワークスペースと一致します。",
            )}
          </p>
        </Card>
      )}
      {dialog}
    </Section>
  );
}
