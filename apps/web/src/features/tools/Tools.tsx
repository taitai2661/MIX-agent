import { t } from "@/app/i18n";
import { api } from "@/app/api";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  BookOpen,
  Brain,
  CalendarClock,
  CircleHelp,
  FileText,
  Globe,
  LayoutGrid,
  ListChecks,
  MousePointerClick,
  Puzzle,
  Search,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  Terminal,
  Wrench,
} from "lucide-react";
import { useMemo, useState } from "react";

import { ErrorBox, Field, Title, useRows } from "@/components/shared";

export type PermissionValue = "allow" | "ask" | "deny";

export const permissionInfo: Record<PermissionValue, { label: string; description: string }> = {
  allow: { label: "常に許可", description: "確認なしで実行できます" },
  ask: { label: "毎回確認", description: "実行前にあなたへ確認します" },
  deny: { label: "使用しない", description: "AgentはこのToolを実行できません" },
};

export type ToolCategoryKey =
  | "web"
  | "files"
  | "terminal"
  | "browser"
  | "knowledge"
  | "memory"
  | "skill"
  | "plan"
  | "schedule"
  | "mcp"
  | "custom";

export const toolCategories: { key: ToolCategoryKey; label: string; icon: any }[] = [
  { key: "web", label: "Web検索・取得", icon: Globe },
  { key: "files", label: "Files", icon: FileText },
  { key: "terminal", label: "Terminal", icon: Terminal },
  { key: "browser", label: "Browser", icon: MousePointerClick },
  { key: "knowledge", label: "Knowledge", icon: BookOpen },
  { key: "memory", label: "Memory", icon: Brain },
  { key: "skill", label: "Skill", icon: Sparkles },
  { key: "plan", label: "Plan", icon: ListChecks },
  { key: "schedule", label: "定期実行", icon: CalendarClock },
  { key: "mcp", label: "MCP", icon: Puzzle },
  { key: "custom", label: "カスタム", icon: Wrench },
];

export function toolCategory(tool: any): ToolCategoryKey {
  const id = tool.id;
  if (tool.source === "mcp") return "mcp";
  if (tool.source !== "builtin") return "custom";
  if (id === "web_search" || id === "web_fetch" || id === "web_fetch_pdf") return "web";
  if (
    id === "read_file" || id === "write_file" || id === "edit_file" ||
    id === "delete_file" || id === "search_files" || id === "create_artifact" ||
    id === "workspace_check" || id.startsWith("files_")
  ) return "files";
  if (id === "run_terminal" || id.startsWith("process_")) return "terminal";
  if (id.startsWith("browser_")) return "browser";
  if (id.startsWith("knowledge_") || id === "web_clip_save") return "knowledge";
  if (id.startsWith("memory_")) return "memory";
  if (id.startsWith("skill_")) return "skill";
  if (id === "update_plan") return "plan";
  if (id.startsWith("schedule_")) return "schedule";
  return "custom";
}

export function resolvedPermission(tool: any, rules: any[] | undefined, agentId: string): PermissionValue {
  const rule = rules?.filter((row) => row.data.agent_id === agentId && row.data.tool_id === tool.id).at(-1);
  return (rule?.data.permission || tool.default_permission || "allow") as PermissionValue;
}

export function riskLabel(risk: string | undefined) {
  return risk === "read" ? "読み取り中心" : risk === "external" ? "外部サービス" : "変更を伴う";
}

function matchesQuery(tool: any, query: string) {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return true;
  return (
    tool.model_name?.toLocaleLowerCase().includes(needle) ||
    tool.id?.toLocaleLowerCase().includes(needle) ||
    tool.description?.toLocaleLowerCase().includes(needle)
  );
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
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<ToolCategoryKey | "">("");
  const [savingToolIds, setSavingToolIds] = useState<Set<string>>(new Set());
  const [bulkSaving, setBulkSaving] = useState(false);
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
  const filtered = useMemo(
    () => permissions.filter((item) => (!category || toolCategory(item.tool) === category) && matchesQuery(item.tool, query)),
    [category, permissions, query],
  );
  const grouped = useMemo(() => {
    const result: Record<ToolCategoryKey, typeof permissions> = {} as Record<ToolCategoryKey, typeof permissions>;
    for (const cat of toolCategories) result[cat.key] = [];
    for (const item of filtered) result[toolCategory(item.tool)].push(item);
    return result;
  }, [filtered]);
  const visibleCount = toolCategories.reduce((sum, cat) => sum + grouped[cat.key].length, 0);

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
  async function bulkApply(permission: PermissionValue) {
    const targets = filtered.map((item) => item.tool);
    if (!targets.length) return;
    const label = permissionInfo[permission].label;
    if (!window.confirm(`「${label}」を表示中の ${targets.length} 件のToolに一括適用しますか？`)) return;
    setBulkSaving(true);
    try {
      for (const tool of targets) {
        setSavingToolIds((current) => new Set(current).add(tool.id));
        try {
          await api("/permission-rules", "POST", { agent_id: agent, tool_id: tool.id, permission });
        } catch (nextError) {
          setSaveErrors((current) => ({ ...current, [tool.id]: nextError }));
        } finally {
          setSavingToolIds((current) => {
            const next = new Set(current);
            next.delete(tool.id);
            return next;
          });
        }
      }
      await qc.invalidateQueries({ queryKey: ["/permission-rules"] });
    } finally {
      setBulkSaving(false);
    }
  }
  return (
    <>
      <Title
        title={t("Tools・権限")}
        sub={t("AIが使える能力と、実行時の確認方法を管理します。")}
      />
      <div className="notice">
        <ShieldAlert size={17} /> {t("Terminalは専用workspace全体を操作できます。ホスト・Docker Socketへのアクセスは提供しません。")}
      </div>
      <div className="notice notice-soft">
        <ShieldCheck size={17} /> {t("読み取り専用の組み込みToolは確認なしで実行されます。ファイル変更・Terminal・Memory/Skill保存などの変更を伴うToolは既定で「毎回確認」です。")}
      </div>
      <section className="tool-agent-panel" aria-label={t("権限の適用先")}>
        <Field label={t("権限を設定するAgent")} hint={t("Agentごとに設定を保存します。標準Agentは個別Agentを選ばない会話に適用されます。")}>
          <select value={agent} onChange={(event) => setAgent(event.target.value)}>
            <option value="">{t("標準Agent")}</option>
            {agents.data?.map((item) => <option key={item.id} value={item.id}>{item.data.name}</option>)}
          </select>
        </Field>
        <p className="tool-agent-current"><ShieldCheck size={16} /><b>{t("適用先:")}</b> {selectedAgent?.data.name || t("標準Agent")}</p>
      </section>
      <section className="permission-overview" aria-label={t("現在のTool権限の内訳")}>
        {(Object.keys(permissionInfo) as PermissionValue[]).map((permission) => <div className={'permission-overview-item ' + permission} key={permission}>
          <span>{t(permissionInfo[permission].label)}</span><b>{counts[permission]} {t("件")}</b><small>{t(permissionInfo[permission].description)}</small>
        </div>)}
      </section>
      <section className="permission-bulk" aria-label={t("一括設定")}>
        <span className="permission-bulk-label">{t("一括設定")}</span>
        <span className="permission-bulk-actions">
          {(Object.keys(permissionInfo) as PermissionValue[]).map((permission) => <button key={permission} className={'bulk-' + permission} type="button" disabled={bulkSaving || !visibleCount} onClick={() => bulkApply(permission)}>{t("すべて" + permissionInfo[permission].label)}</button>)}
        </span>
        <small className="permission-bulk-hint">{t("表示中のToolへ適用します。Agentごとに保存されます。")}</small>
      </section>
      <section className="tool-filters" aria-label={t("Toolを絞り込む")}>
        <label className="tool-search">
          <Search size={15} />
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("名前や説明で検索")} />
        </label>
        <div className="tool-chips" role="tablist" aria-label={t("Toolカテゴリ")}>
          <button type="button" className={'tool-chip' + (category === "" ? " active" : "")} onClick={() => setCategory("")}><LayoutGrid size={13} />{t("すべて")}<span>{permissions.length}</span></button>
          {toolCategories.map(({ key, label, icon: Icon }) => <button type="button" key={key} className={'tool-chip' + (category === key ? " active" : "")} onClick={() => setCategory(key)}><Icon size={13} />{t(label)}<span>{grouped[key].length}</span></button>)}
        </div>
      </section>
      <ErrorBox error={tools.error || rules.error || agents.error || settings.error} />
      <section className="tool-groups" aria-label={t("Toolごとの権限")}>
        {toolCategories.map(({ key, label, icon: Icon }) => {
          const items = grouped[key];
          if (!items.length) return null;
          const breakdown = items.reduce<Record<PermissionValue, number>>(
            (total, item) => ({ ...total, [item.permission]: total[item.permission] + 1 }),
            { allow: 0, ask: 0, deny: 0 },
          );
          return (
            <details className="tool-group" key={key} open>
              <summary>
                <span className="tool-group-title"><Icon size={15} /><b>{t(label)}</b><small>{items.length} {t("件")}</small></span>
                <span className="tool-group-counts">
                  <span className="allow">{breakdown.allow} {t("許可")}</span>
                  <span className="ask">{breakdown.ask} {t("毎回確認")}</span>
                  <span className="deny">{breakdown.deny} {t("拒否")}</span>
                </span>
              </summary>
              <div className="tool-group-body">
                {items.map(({ tool, permission }) => (
                  <article className={'tool-card permission-' + permission} key={tool.id} aria-busy={savingToolIds.has(tool.id)}>
                    <div className="tool-card-main">
                      <div className="tool-card-heading"><b>{tool.model_name}</b><span className={'permission-badge ' + permission}>{t(permissionInfo[permission].label)}</span></div>
                      <p>{tool.description}</p>
                      <div className="tool-meta">
                        <span>{tool.source === "builtin" ? t("組み込み") : tool.source}</span>
                        <span>{t(riskLabel(tool.risk))}</span>
                        {tool.default_permission && <span>{t("標準:")} {t(permissionInfo[tool.default_permission as PermissionValue]?.label || "")}</span>}
                      </div>
                    </div>
                    <div className="tool-card-side">
                      <label className="check"><input type="checkbox" checked={settings.data?.data.tool_settings?.[tool.id]?.enabled !== false} onChange={(event) => updateEnabled(tool.id, event.target.checked)} /> {t("有効")}</label>
                      <div className="permission-segment" role="radiogroup" aria-label={t("実行時の扱い")}>
                        {(Object.keys(permissionInfo) as PermissionValue[]).map((value) => <button key={value} type="button" role="radio" aria-checked={permission === value} className={value + (permission === value ? " active" : "")} disabled={savingToolIds.has(tool.id) || bulkSaving} onClick={() => updatePermission(tool.id, value)}>{t(permissionInfo[value].label)}</button>)}
                      </div>
                      <small className="tool-card-note">{savingToolIds.has(tool.id) ? t("保存しています…") : t(permissionInfo[permission].description)}</small>
                      <ErrorBox error={saveErrors[tool.id]} />
                    </div>
                  </article>
                ))}
              </div>
            </details>
          );
        })}
        {!tools.isLoading && !visibleCount && <div className="empty"><CircleHelp size={25} /><p>{t("条件に一致するToolはありません。")}</p></div>}
      </section>
    </>
  );
}