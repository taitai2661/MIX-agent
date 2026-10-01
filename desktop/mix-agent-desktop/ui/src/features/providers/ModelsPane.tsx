import { useQueryClient } from "@tanstack/react-query";
import { Sparkles } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { DataTable, Section, td, tdStrong } from "@/components/layout";
import { EmptyState, TableSkeleton } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { YesNo } from "@/components/ui/status";
import { MODELS_QUERY_KEY, useModels, useProviders } from "@/lib/queries";
import type { ModelView } from "@/lib/types";

export function ModelsPane() {
  const queryClient = useQueryClient();
  const models = useModels();
  const providers = useProviders();
  const verifyTools = (id: string) =>
    api.verifyTools(id).then(() => queryClient.invalidateQueries({ queryKey: MODELS_QUERY_KEY }));

  const list = models.data ?? [];

  return (
    <Section>
      {models.isPending ? (
        // "No models yet" while the query is still in flight was indistinguishable
        // from the real empty state, so the pane flashed an empty table on every
        // visit and repainted a frame later.
        <TableSkeleton />
      ) : list.length === 0 ? (
        <EmptyState
          icon={<Sparkles size={22} />}
          title={t("モデルが未設定です。先にプロバイダーを追加してください。")}
        />
      ) : (
        <DataTable
          columns={[t("ラベル"), t("Provider"), t("Context"), t("Vision"), t("Tools"), t("Reasoning"), ""]}
          rows={list}
          rowKey={(m: ModelView) => m.id}
        >
          {(m) => {
            const provider = providers.data?.find((p) => p.id === m.provider_id);
            return (
              <>
                <td className={tdStrong}>{m.label}</td>
                <td className={td}>{provider?.label ?? m.provider_id}</td>
                <td className={`${td} font-mono`}>{m.context_window.toLocaleString()}</td>
                <td className={td}><YesNo value={m.supports_vision} /></td>
                <td className={td}><YesNo value={m.supports_tool_calling} /></td>
                <td className={td}><YesNo value={m.supports_reasoning} /></td>
                <td className={`${td} w-24`}>
                  <Button size="sm" onClick={() => void verifyTools(m.id)}>
                    <Sparkles size={11} /> {t("能力を確認")}
                  </Button>
                </td>
              </>
            );
          }}
        </DataTable>
      )}
    </Section>
  );
}
