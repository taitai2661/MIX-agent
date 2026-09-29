import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Sparkles } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { DataTable, Section, td, tdStrong } from "@/components/layout";
import { EmptyState } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { ModelView } from "@/lib/types";

function YesNo({ value }: { value: boolean }) {
  return value
    ? <Badge tone="success" size="sm"><Check size={10} /></Badge>
    : <Badge size="sm">—</Badge>;
}

export function ModelsPane() {
  const queryClient = useQueryClient();
  const models = useQuery({ queryKey: ["models"], queryFn: api.listModels });
  const providers = useQuery({ queryKey: ["providers"], queryFn: api.listProviders });
  const verifyTools = (id: string) =>
    api.verifyTools(id).then(() => queryClient.invalidateQueries({ queryKey: ["models"] }));

  const list = models.data ?? [];

  return (
    <Section>
      {list.length === 0 ? (
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
                    <Sparkles size={11} /> {t("Verify")}
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
