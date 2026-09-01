import { api } from "@/app/api";
import { Title, useRows } from "@/components/shared";
import { useQuery } from "@tanstack/react-query";
import { ArrowRight, BarChart3, Brain, Database, Globe, Server, ShieldCheck, Sparkles } from "lucide-react";
import { Link } from "react-router-dom";

const cards = [
  ["providers", "AI Providers", "接続済みのAIサービス", Server],
  ["models", "モデル", "会話で使うモデル", Sparkles],
  ["memory", "Memory", "記憶の形成・想起・管理", Brain],
  ["tools", "Tools・権限", "Agentの実行権限", ShieldCheck],
  ["browser", "Browser", "Webページの操作と許可先", Globe],
  ["web-search", "Web検索", "検索サービスと結果数", Globe],
  ["mcp", "MCP", "外部サービスとの接続", Globe],
  ["account", "アカウント・安全性", "認証とログイン履歴", ShieldCheck],
  ["backups", "バックアップ", "データを安全に保管", Database],
  ["statistics", "統計", "モデルの成功率と失敗傾向", BarChart3],
] as const;

export function SettingsOverview() {
  const providers = useRows("/providers");
  const models = useRows("/models");
  const tools = useQuery<any[]>({ queryKey: ["/tools"], queryFn: () => api("/tools") });
  const mcp = useRows("/mcp/connections");
  const settings = useQuery<any>({ queryKey: ["/settings"], queryFn: () => api("/settings") });
  const counts: Record<string, string> = {
    providers: `${providers.data?.length || 0} 件`,
    models: `${models.data?.length || 0} 件`,
    tools: `${tools.data?.length || 0} 件`,
    mcp: `${mcp.data?.filter((row) => row.data.enabled).length || 0} 件接続中`,
    account: "変更・確認",
    backups: "作成・復元",
    statistics: "30日間",
  };
  const defaultModel = models.data?.find((row) => row.id === settings.data?.data.default_model_id);
  return <div className="settings-overview">
    <Title title="設定の概要" sub="AIの接続、使い方、安全性をここから管理できます。" />
    <section className="settings-hero">
      <div><p className="eyebrow">YOUR WORKSPACE</p><h2>いつでも、あなたの使い方に。</h2><p>接続したAIと権限はこの環境だけで管理されます。</p></div>
      <Link to="/settings/providers" className="button btn-primary">Providerを追加 <ArrowRight size={16} /></Link>
    </section>
    <section className="settings-summary" aria-label="現在の設定">
      <div><span>既定モデル</span><b>{defaultModel?.data.name || defaultModel?.data.model_id || "未設定"}</b></div>
      <div><span>接続状態</span><b>{providers.data?.length ? "準備完了" : "Providerを追加"}</b></div>
      <div><span>テーマ</span><b>表示設定で変更</b></div>
    </section>
    <div className="settings-overview-grid">
      {cards.map(([path, title, description, Icon]) => <Link key={path} to={"/settings/" + path} className="settings-overview-card">
        <span className="settings-card-icon"><Icon size={20} /></span><span className="settings-card-copy"><b>{title}</b><small>{description}</small></span><strong>{counts[path]}</strong><ArrowRight size={17} />
      </Link>)}
    </div>
  </div>;
}
