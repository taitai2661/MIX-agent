import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Wrench } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { DataTable, Section, td, tdStrong } from "@/components/layout";
import { EmptyState } from "@/components/shared";
import { Badge, type BadgeProps } from "@/components/ui/badge";
import { Select } from "@/components/ui/input";
import type { PermissionRuleView, ToolView } from "@/lib/types";

function permissionTone(p: string): NonNullable<BadgeProps["tone"]> {
  if (p === "allow") return "success";
  if (p === "deny") return "danger";
  return "warning";
}

export function ToolsPane() {
  const queryClient = useQueryClient();
  const tools = useQuery({ queryKey: ["tools"], queryFn: api.listTools });
  const rules = useQuery({ queryKey: ["permission_rules"], queryFn: api.listPermissionRules });
  const setRule = useMutation({
    mutationFn: (input: { tool_id: string; scope: string; decision: string; pattern: string | null }) =>
      api.setPermissionRule(input),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["permission_rules"] }),
  });

  const decisionFor = (toolId: string) =>
    rules.data?.find((r: PermissionRuleView) => r.tool_id === toolId)?.decision ?? "ask";

  const list = tools.data ?? [];

  return (
    <Section>
      {list.length === 0 ? (
        <EmptyState icon={<Wrench size={22} />} title={t("ツール")} sub="まだ登録されていません。" />
      ) : (
        <DataTable
          columns={["ID", "説明", "既定権限", "権限ルール"]}
          rows={list}
          rowKey={(tool: ToolView) => tool.id}
        >
          {(tool) => (
            <>
              <td className={`${tdStrong} font-mono`}>{tool.id}</td>
              <td className={`${td} max-w-96`}>
                <Wrench size={11} className="me-1 inline-block align-[-1px] text-faint" />
                {tool.description}
              </td>
              <td className={td}>
                <span className="inline-flex items-center gap-1.5">
                  <Badge tone={permissionTone(tool.default_permission)} size="sm">
                    {tool.mixed
                      ? `${tool.default_permission} / ask`
                      : tool.default_permission}
                  </Badge>
                  {tool.mixed && (
                    <span
                      className="text-2xs text-faint"
                      title="読み取りは許可、書き込み・削除は承認"
                    >
                      read / write
                    </span>
                  )}
                </span>
              </td>
              <td className={td}>
                <Select
                  className="w-30"
                  value={decisionFor(tool.id)}
                  onChange={(e) =>
                    setRule.mutate({
                      tool_id: tool.id, scope: "global", decision: e.target.value, pattern: null,
                    })
                  }
                >
                  <option value="allow">allow</option>
                  <option value="ask">ask</option>
                  <option value="deny">deny</option>
                </Select>
              </td>
            </>
          )}
        </DataTable>
      )}
    </Section>
  );
}
