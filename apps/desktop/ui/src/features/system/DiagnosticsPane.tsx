import { useQuery } from "@tanstack/react-query";
import { Activity, Lock } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { Card, Row, Section } from "@/components/layout";
import { Alert } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { ErrorAlert, ListSkeleton } from "@/components/shared";

function Check({ label, ok, children }: { label: string; ok: boolean; children?: React.ReactNode }) {
  return (
    <Card>
      <Row align="between">
        <strong className="text-sm font-medium text-fg">{label}</strong>
        <Badge tone={ok ? "success" : "danger"} size="sm">{ok ? t("OK") : t("失敗")}</Badge>
      </Row>
      {children}
    </Card>
  );
}

export function DiagnosticsPane() {
  const diagnostics = useQuery({
    queryKey: ["diagnostics"],
    queryFn: api.diagnostics,
    refetchInterval: 2000,
    // A background tab doesn't need a 2s poll; pause it when hidden.
    refetchIntervalInBackground: false,
  });
  const d = diagnostics.data;
  return (
    <Section>
      {diagnostics.isPending ? (
        <ListSkeleton />
      ) : diagnostics.isError || !d ? (
        // The old `isPending || !d` check rendered the skeleton forever on a
        // failed query, so a broken diagnostics command looked like a hang.
        <ErrorAlert error={diagnostics.error} />
      ) : (
        <>
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
                    : t("未暗号化（メモリ内 + Keychainのみ）")}
                </span>
                <span className="text-xs text-muted">
                  {d.db_key_present === true
                    ? t("マスターキーはOSのKeychainにあります")
                    : d.db_key_present === false
                      ? t("マスターキーは初回利用時に作成されます")
                      : t("Keychainの状態は不明")}
                </span>
              </div>
              <Badge
                tone={d.db_encrypted ? "success" : "warning"}
                size="sm"
              >
                {d.db_encrypted ? t("暗号化済み") : t("保留中")}
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
              <Badge size="sm">{t("プロバイダー")}: {d.provider_count}</Badge>
              <Badge size="sm">{t("モデル")}: {d.model_count}</Badge>
              <Badge size="sm">{t("メモリー")}: {d.memory_count}</Badge>
              <Badge size="sm">{t("スキル")}: {d.skill_count}</Badge>
              <Badge tone={d.active_runs > 0 ? "warning" : "neutral"} size="sm">
                {t("実行中のRun")}: {d.active_runs}
              </Badge>
            </Row>
          </Card>
        </>
      )}
    </Section>
  );
}
