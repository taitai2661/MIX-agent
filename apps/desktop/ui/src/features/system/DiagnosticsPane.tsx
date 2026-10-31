import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Activity,
  Copy,
  Download,
  FileText,
  Lock,
  Terminal,
} from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { Card, Row, Section } from "@/components/layout";
import { Alert } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ErrorAlert, ListSkeleton } from "@/components/shared";

function Check({
  label,
  ok,
  children,
}: {
  label: string;
  ok: boolean;
  children?: React.ReactNode;
}) {
  return (
    <Card>
      <Row align="between">
        <strong className="text-sm font-medium text-fg">{label}</strong>
        <Badge tone={ok ? "success" : "danger"} size="sm">
          {ok ? t("OK") : t("失敗")}
        </Badge>
      </Row>
      {children}
    </Card>
  );
}

export function DiagnosticsPane() {
  const [copied, setCopied] = useState(false);
  const [showLogs, setShowLogs] = useState(false);
  const diagnostics = useQuery({
    queryKey: ["diagnostics"],
    queryFn: api.diagnostics,
    refetchInterval: 2000,
    refetchIntervalInBackground: false,
  });

  const recentLogs = useQuery({
    queryKey: ["recent_logs"],
    queryFn: () => api.getRecentLogs(200),
    enabled: showLogs,
  });

  const d = diagnostics.data;

  const handleCopyReport = async () => {
    try {
      const exported = await api.exportDiagnostics();
      const text = JSON.stringify(exported, null, 2);
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // fallback
    }
  };

  const handleDownloadReport = async () => {
    try {
      const exported = await api.exportDiagnostics();
      const blob = new Blob([JSON.stringify(exported, null, 2)], {
        type: "application/json",
      });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `mix-agent-diagnostics-${new Date().toISOString().slice(0, 10)}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch {
      // fallback
    }
  };

  return (
    <Section>
      {diagnostics.isPending ? (
        <ListSkeleton />
      ) : diagnostics.isError || !d ? (
        <ErrorAlert error={diagnostics.error} />
      ) : (
        <>
          <div className="flex items-center justify-between pb-1">
            <h3 className="text-sm font-semibold text-fg">
              {t("システム診断")}
            </h3>
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setShowLogs(!showLogs)}
              >
                <Terminal size={13} className="mr-1" />
                {showLogs ? t("ログを隠す") : t("最近のログ")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void handleCopyReport()}
              >
                <Copy size={13} className="mr-1" />
                {copied ? t("コピー完了") : t("診断をコピー")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void handleDownloadReport()}
              >
                <Download size={13} className="mr-1" />
                {t("書き出し")}
              </Button>
            </div>
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            <Check label={t("ストレージ")} ok={d.db_ok}>
              <code className="break-all text-xs text-faint">{d.data_dir}</code>
            </Check>
            <Check label={t("シークレット (Keychain)")} ok={d.secrets_ok} />
            <Check label={t("エンジン")} ok={d.engine_ok} />
          </div>

          <Card title={t("データベースの暗号化")} icon={<Lock size={13} />}>
            <Row align="between" className="gap-2">
              <div className="flex flex-col gap-0.5 text-xs">
                <span className="text-fg">
                  {d.db_encrypted
                    ? t("保存時は暗号化済み")
                    : t("未暗号化（平文SQLite）")}
                </span>
                <span className="text-xs text-muted">
                  {d.db_key_present === true
                    ? t("暗号化用キーはKeychainに準備済み（未使用）")
                    : d.db_key_present === false
                      ? t("暗号化用キーは未作成")
                      : t("Keychainの状態は不明")}
                </span>
              </div>
              <Badge tone={d.db_encrypted ? "success" : "warning"} size="sm">
                {d.db_encrypted ? t("暗号化済み") : t("未対応")}
              </Badge>
            </Row>
            {!d.db_encrypted && (
              <Alert tone="warning">
                {t(
                  "Database at-rest encryption is not enabled in this build. Conversation logs, run history and tool arguments are stored in plaintext; rely on disk-level encryption (FileVault / BitLocker / LUKS) until the SQLCipher migration lands.",
                )}
              </Alert>
            )}
          </Card>

          <Card title={t("件数")} icon={<Activity size={13} />}>
            <Row className="gap-1.5">
              <Badge size="sm">
                {t("プロバイダー")}: {d.provider_count}
              </Badge>
              <Badge size="sm">
                {t("モデル")}: {d.model_count}
              </Badge>
              <Badge size="sm">
                {t("メモリー")}: {d.memory_count}
              </Badge>
              <Badge size="sm">
                {t("スキル")}: {d.skill_count}
              </Badge>
              <Badge tone={d.active_runs > 0 ? "warning" : "neutral"} size="sm">
                {t("実行中のRun")}: {d.active_runs}
              </Badge>
            </Row>
          </Card>

          {showLogs && (
            <Card title={t("最近の実行ログ")} icon={<FileText size={13} />}>
              {recentLogs.isPending ? (
                <div className="py-2 text-xs text-muted">
                  {t("ログ読み込み中...")}
                </div>
              ) : recentLogs.isError ? (
                <ErrorAlert error={recentLogs.error} />
              ) : (
                <pre className="max-h-60 overflow-y-auto whitespace-pre-wrap rounded bg-inset p-2 font-mono text-[11px] text-muted">
                  {recentLogs.data || t("ログはありません")}
                </pre>
              )}
            </Card>
          )}
        </>
      )}
    </Section>
  );
}
