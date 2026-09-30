import { useQuery } from "@tanstack/react-query";
import { Activity, Lock } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { Card, Row, Section } from "@/components/layout";
import { Alert, Skeleton } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";

function Check({ label, ok, children }: { label: string; ok: boolean; children?: React.ReactNode }) {
  return (
    <Card>
      <Row align="between">
        <strong className="text-sm font-medium text-fg">{label}</strong>
        <Badge tone={ok ? "success" : "danger"} size="sm">{ok ? t("OK") : t("Fail")}</Badge>
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
  });
  const d = diagnostics.data;
  return (
    <Section>
      {!d ? (
        <Skeleton className="h-20 w-full" />
      ) : (
        <>
          <div className="grid gap-3 sm:grid-cols-2">
            <Check label={t("Storage")} ok={d.db_ok}>
              <code className="break-all text-2xs text-faint">{d.data_dir}</code>
            </Check>
            <Check label={t("Secrets (Keychain)")} ok={d.secrets_ok} />
            <Check label={t("Engine")} ok={d.engine_ok} />
          </div>
          <Card title={t("Database Encryption")} icon={<Lock size={13} />}>
            <Row align="between" className="gap-2">
              <div className="flex flex-col gap-0.5 text-xs">
                <span className="text-fg">
                  {d.db_encrypted
                    ? t("Encrypted at rest")
                    : t("Not encrypted (in-memory + Keychain only)")}
                </span>
                <span className="text-2xs text-faint">
                  {d.db_key_present === true
                    ? t("Master key is present in the OS Keychain")
                    : d.db_key_present === false
                      ? t("Master key will be created on first use")
                      : t("Keychain status unknown")}
                </span>
              </div>
              <Badge
                tone={d.db_encrypted ? "success" : "warning"}
                size="sm"
              >
                {d.db_encrypted ? t("Encrypted") : t("Pending")}
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
          <Card title={t("Counts")} icon={<Activity size={13} />}>
            <Row className="gap-1.5">
              <Badge size="sm">{t("Providers")}: {d.provider_count}</Badge>
              <Badge size="sm">{t("Models")}: {d.model_count}</Badge>
              <Badge size="sm">{t("Memories")}: {d.memory_count}</Badge>
              <Badge size="sm">{t("Skills")}: {d.skill_count}</Badge>
              <Badge tone={d.active_runs > 0 ? "warning" : "neutral"} size="sm">
                {t("Active Runs")}: {d.active_runs}
              </Badge>
            </Row>
          </Card>
        </>
      )}
    </Section>
  );
}
