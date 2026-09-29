import { useQuery } from "@tanstack/react-query";
import { Activity } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { Card, Row, Section } from "@/components/layout";
import { Skeleton } from "@/components/ui/alert";
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
