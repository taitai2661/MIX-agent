/// Visible failure state for the backend health poll.
///
/// A red 8px dot is not a recovery affordance: when the local service died the
/// only visible change was a colour, and the user had to guess whether to wait,
/// restart, or open diagnostics. This banner appears only on failure, names the
/// problem, and offers a retry.

import { AlertTriangle, RefreshCw } from "lucide-react";
import { t } from "@/lib/i18n";
import { errorMessage } from "@/lib/errors";
import { useHealth } from "@/lib/health";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";

export function HealthBanner() {
  const health = useHealth();
  if (!health.isError) return null;
  const detail = errorMessage(health.error);
  return (
    <div
      role="alert"
      className="flex shrink-0 items-center gap-2 border-b border-danger/25 bg-danger-soft px-4 py-1.5 text-xs text-danger"
    >
      <AlertTriangle size={13} aria-hidden="true" className="shrink-0" />
      <span className="font-medium">{t("バックエンドに接続できません")}</span>
      {detail && (
        <span className="min-w-0 truncate text-2xs opacity-80" title={detail}>
          {detail}
        </span>
      )}
      <span className="flex-1" />
      <Button
        size="sm"
        variant="ghost"
        className="h-6 shrink-0 gap-1 px-2 text-2xs text-danger hover:bg-danger/10 hover:text-danger"
        disabled={health.isFetching}
        onClick={() => void health.refetch()}
      >
        {health.isFetching ? (
          <Spinner size={11} />
        ) : (
          <RefreshCw size={11} aria-hidden="true" />
        )}
        {t("再試行")}
      </Button>
    </div>
  );
}
