import { Radar } from "lucide-react";
import { t } from "@/lib/i18n";
import { Card } from "@/components/layout";
import { LocalRuntimePanel } from "./LocalRuntimePanel";
import { LocalRuntimeScan } from "./LocalRuntimeScan";

/** Settings → Providers: the local-runtime sweep plus the manual Base URL
 * field, which is useful once someone already knows they run a custom port.
 *
 * The sweep itself lives in `LocalRuntimeScan` because the first-run wizard
 * reuses it — this only supplies the surrounding chrome and the copy that
 * assumes the user came here deliberately rather than being asked to set the
 * app up. */
export function LocalDiscoveryCard() {
  return (
    <Card title={t("ローカル LLM を検出")} icon={<Radar size={13} />}>
      <LocalRuntimeScan
        intro={t(
          "LM Studio や Ollama などのローカル推論サーバーを自動で探します。",
        )}
        emptyHint={t(
          "この PC で検出できるランタイムはありません。下の手動指定か、上のプリセットから追加してください。",
        )}
        showManualProbe
      />
    </Card>
  );
}

/** Settings → Providers: lifecycle and model management for already detected
 * runtimes. Kept separate from the sweep so the import table stays focused on
 * "which runtime should become a provider". */
export function LocalRuntimeManagementCard() {
  return <LocalRuntimePanel />;
}
