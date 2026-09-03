import { api } from "@/app/api";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleHelp, ShieldAlert, ShieldCheck } from "lucide-react";
import { useMemo, useState } from "react";

import { ErrorBox, Field, Title, useRows } from "@/components/shared";

export type PermissionValue = "allow" | "ask" | "deny";

export const permissionInfo: Record<PermissionValue, { label: string; description: string }> = {
  allow: { label: "常に許可", description: "確認なしで実行できます" },
  ask: { label: "毎回確認", description: "実行前にあなたへ確認します" },
  deny: { label: "使用しない", description: "AgentはこのToolを実行できません" },
};

export function resolvedPermission(tool: any, rules: any[] | undefined, agentId: string): PermissionValue {
  const rule = rules?.filter((row) => row.data.agent_id === agentId && row.data.tool_id === tool.id).at(-1);
  return (rule?.data.permission || tool.default_permission || "ask") as PermissionValue;
}

export function riskLabel(risk: string | undefined) {
  return risk === "read" ? "読み取り中心" : risk === "external" ? "外部サービス" : "変更を伴う";
}

export function Tools() {
  const tools = useQuery<any[]>({
      queryKey: ["/tools"],
      queryFn: () => api("/tools"),
    }),
    settings = useQuery<any>({ queryKey: ["/settings"], queryFn: () => api("/settings") }),
    rules = useRows("/permission-rules"),
    agents = useRows("/agents"),
    qc = useQueryClient();
  const [agent, setAgent] = useState("");
  const [savingToolIds, setSavingToolIds] = useState<Set<string>>(new Set());
  const [saveErrors, setSaveErrors] = useState<Record<string, unknown>>({});
  const selectedAgent = agents.data?.find((item) => item.id === agent);
  const permissions = useMemo(
    () => (tools.data || []).map((tool) => ({ tool, permission: resolvedPermission(tool, rules.data, agent) })),
    [agent, rules.data, tools.data],
  );
  const counts = permissions.reduce<Record<PermissionValue, number>>(
    (total, item) => ({ ...total, [item.permission]: total[item.permission] + 1 }),
    { allow: 0, ask: 0, deny: 0 },
  );
  async function updatePermission(toolId: string, permission: PermissionValue) {
    setSavingToolIds((current) => new Set(current).add(toolId));
    setSaveErrors((current) => ({ ...current, [toolId]: null }));
    try {
      await api("/permission-rules", "POST", { agent_id: agent, tool_id: toolId, permission });
      await qc.invalidateQueries({ queryKey: ["/permission-rules"] });
    } catch (nextError) {
      setSaveErrors((current) => ({ ...current, [toolId]: nextError }));
    } finally {
      setSavingToolIds((current) => {
        const next = new Set(current);
        next.delete(toolId);
        return next;
      });
    }
  }
  async function updateEnabled(toolId: string, enabled: boolean) {
    if (!settings.data) return;
    const current = settings.data.data.tool_settings || {};
    const data = settings.data.data;
    await api("/settings", "PUT", { default_model_id: data.default_model_id || "", auto_model_ids: data.auto_model_ids || null, auto_retry_count: data.auto_retry_count ?? 3, setup_complete: data.setup_complete ?? false, browser_enabled: data.browser_enabled !== false, web_search_enabled: data.web_search_enabled !== false, web_search_backend: data.web_search_backend || "ddgs", web_search_count: data.web_search_count ?? 5, searxng_url: data.searxng_url || "", allowed_domains: data.allowed_domains || [], tool_settings: { ...current, [toolId]: { ...(current[toolId] || {}), enabled } }, brave_api_key: null, tavily_api_key: null, exa_api_key: null, serper_api_key: null });
    await qc.invalidateQueries({ queryKey: ["/settings"] });
  }
  return (
    <>
      <Title
        title="Tools・権限"
        sub="AIが使える能力と、実行時の確認方法を管理します。"
      />
      <div className="notice">
        <ShieldAlert size={17} /> Terminalは専用workspace全体を操作できます。ホスト・Docker Socketへのアクセスは提供しません。
      </div>
      <section className="tool-agent-panel" aria-label="権限の適用先">
        <Field label="権限を設定するAgent" hint="Agentごとに設定を保存します。標準Agentは個別Agentを選ばない会話に適用されます。">
          <select value={agent} onChange={(event) => setAgent(event.target.value)}>
            <option value="">標準Agent</option>
            {agents.data?.map((item) => <option key={item.id} value={item.id}>{item.data.name}</option>)}
          </select>
        </Field>
        <p className="tool-agent-current"><ShieldCheck size={16} /><b>適用先:</b> {selectedAgent?.data.name || "標準Agent"}</p>
      </section>
      <section className="permission-overview" aria-label="現在のTool権限の内訳">
        {(Object.keys(permissionInfo) as PermissionValue[]).map((permission) => <div className={'permission-overview-item ' + permission} key={permission}>
          <span>{permissionInfo[permission].label}</span><b>{counts[permission]} 件</b><small>{permissionInfo[permission].description}</small>
        </div>)}
      </section>
      <ErrorBox error={tools.error || rules.error || agents.error || settings.error} />
      <section className="tool-list" aria-label="Toolごとの権限">
        {permissions.map(({ tool, permission }) => <article className={'tool-card permission-' + permission} key={tool.id} aria-busy={savingToolIds.has(tool.id)}>
          <div className="tool-card-main"><div className="tool-card-heading"><b>{tool.model_name}</b><span className={'permission-badge ' + permission}>{permissionInfo[permission].label}</span></div><p>{tool.description}</p><div className="tool-meta"><span>{tool.source === "builtin" ? "組み込み" : tool.source}</span><span>{riskLabel(tool.risk)}</span></div></div>
          <label className="check"><input type="checkbox" checked={settings.data?.data.tool_settings?.[tool.id]?.enabled !== false} onChange={(event) => updateEnabled(tool.id, event.target.checked)} /> 有効</label>
          <div className="tool-card-control"><label htmlFor={'permission-' + tool.id}>実行時の扱い</label><select id={'permission-' + tool.id} value={permission} disabled={savingToolIds.has(tool.id)} onChange={(event) => updatePermission(tool.id, event.target.value as PermissionValue)}>{(Object.keys(permissionInfo) as PermissionValue[]).map((value) => <option key={value} value={value}>{permissionInfo[value].label}</option>)}</select><small>{savingToolIds.has(tool.id) ? "保存しています…" : permissionInfo[permission].description}</small><ErrorBox error={saveErrors[tool.id]} /></div>
        </article>)}
        {!tools.isLoading && !permissions.length && <div className="empty"><CircleHelp size={25} /><p>利用可能なToolはありません。</p></div>}
      </section>
    </>
  );
}
