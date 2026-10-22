import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Wrench } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { DataTable, Section, td, tdStrong } from "@/components/layout";
import {
  EmptyState,
  ErrorBox,
  ErrorBadge,
  TableSkeleton,
} from "@/components/shared";
import { SectionHeading } from "@/components/common";
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
  const rules = useQuery({
    queryKey: ["permission_rules"],
    queryFn: api.listPermissionRules,
  });
  const setRule = useMutation({
    mutationFn: (input: {
      tool_id: string;
      scope: string;
      decision: string;
      pattern: string | null;
      workspace_id?: string | null;
    }) => api.setPermissionRule(input),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["permission_rules"] }),
  });

  const globalDecisionFor = (toolId: string) =>
    rules.data?.find(
      (r: PermissionRuleView) => r.tool_id === toolId && r.scope === "global",
    )?.decision ?? "ask";

  const list = tools.data ?? [];
  const allRules = rules.data ?? [];

  return (
    <Section>
      {tools.isPending ? (
        <TableSkeleton />
      ) : tools.isError ? (
        <ErrorBox error={tools.error} />
      ) : list.length === 0 ? (
        <EmptyState
          icon={<Wrench size={22} />}
          title={t("ツール")}
          sub={t("まだ登録されていません。")}
        />
      ) : (
        <>
          <DataTable
            columns={["ID", t("説明"), t("既定権限"), t("権限ルール")]}
            rows={list}
            rowKey={(tool: ToolView) => tool.id}
          >
            {(tool) => (
              <>
                <td className={`${tdStrong} font-mono`}>{tool.id}</td>
                <td className={`${td} max-w-96`}>
                  <Wrench
                    size={12}
                    className="me-1 inline-block align-[-1px] text-faint"
                  />
                  {tool.description}
                </td>
                <td className={td}>
                  <span className="inline-flex items-center gap-1.5">
                    <Badge
                      tone={permissionTone(tool.default_permission)}
                      size="sm"
                    >
                      {tool.mixed
                        ? `${tool.default_permission} / ask`
                        : tool.default_permission}
                    </Badge>
                    {tool.mixed && (
                      <span
                        className="text-xs text-faint"
                        title={t("読み取りは許可、書き込み・削除は承認")}
                      >
                        read / write
                      </span>
                    )}
                  </span>
                </td>
                <td className={td}>
                  <Select
                    className="w-30"
                    value={globalDecisionFor(tool.id)}
                    onChange={(e) =>
                      setRule.mutate({
                        tool_id: tool.id,
                        scope: "global",
                        decision: e.target.value,
                        pattern: null,
                        workspace_id: null,
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
          {/* A failed permission write used to snap the select back with no
              explanation: the rule the user just chose silently did not stick. */}
          {setRule.isError && <ErrorBadge error={setRule.error} />}
          {rules.isError && <ErrorBox error={rules.error} />}
          {allRules.length > 0 && (
            <div className="flex flex-col gap-1">
              <SectionHeading>{t("スコープ付きルール")}</SectionHeading>
              <ul className="flex flex-col divide-y divide-line rounded-xl border border-line bg-surface shadow-xs">
                {allRules.map((rule) => (
                  <li
                    key={rule.id}
                    className="flex items-center gap-3 px-3 py-1.5 text-xs"
                  >
                    <span className="font-mono text-fg">{rule.tool_id}</span>
                    <Badge tone={permissionTone(rule.decision)} size="sm">
                      {rule.scope} · {rule.decision}
                    </Badge>
                    <span className="font-mono text-muted">
                      {rule.pattern ? `pattern: ${rule.pattern}` : "*"}
                    </span>
                    {rule.workspace_id && (
                      <span className="font-mono text-faint">
                        ws {rule.workspace_id.slice(0, 8)}
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </>
      )}
    </Section>
  );
}
