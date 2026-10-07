import { t } from "@/app/i18n";
import { api } from "@/app/api";
import { Title, useRows } from "@/components/shared";
import { useQuery } from "@tanstack/react-query";
import { Activity, ArrowRight, BarChart3, Brain, Coins, Database, Globe, Network, Server, Settings2, ShieldCheck, Sparkles } from "lucide-react";
import { Link } from "react-router-dom";

const groups = [
  {
    label: "AI",
    cards: [
      ["providers", "AI Providers", "接続済みのAIサービス", Server],
      ["models", "モデル", "会話で使うモデル", Sparkles],
      ["auto", "Auto", "自動選択の候補と再試行", Sparkles],
      ["memory", "Associative Memory", "記憶の形成・想起・管理", Brain],
    ],
  },
  {
    label: "ツールと連携",
    cards: [
      ["tools", "Tools・権限", "Agentの実行権限", ShieldCheck],
      ["mcp", "MCP", "外部サービスとの接続", Globe],
      ["browser", "Browser", "Webページの操作と許可先", Globe],
      ["web-search", "Web検索", "検索サービスと結果数", Globe],
    ],
  },
  {
    label: "基本設定",
    cards: [
      ["general", "一般", "表示テーマ・言語・既定モデル", Settings2],
    ],
  },
  {
    label: "セキュリティとデータ",
    cards: [
      ["network", "通信とネットワーク", "Runnerの通信許可先", Network],
      ["account", "アカウント・安全性", "認証とログイン履歴", ShieldCheck],
      ["backups", "バックアップ", "データを安全に保管", Database],
    ],
  },
  {
    label: "運用",
    cards: [
      ["usage", "使用量", "トークン使用量と概算コスト", Coins],
      ["statistics", "統計", "モデルの成功率と失敗傾向", BarChart3],
      ["diagnostics", "障害診断", "サービス状態と最近の失敗", Activity],
    ],
  },
] as const;

export function SettingsOverview() {
  const providers = useRows("/providers");
  const models = useRows("/models");
  const tools = useQuery<any[]>({ queryKey: ["/tools"], queryFn: () => api("/tools") });
  const mcp = useRows("/mcp/connections");
  const settings = useQuery<any>({ queryKey: ["/settings"], queryFn: () => api("/settings") });
  const domains = settings.data?.data.allowed_domains?.length || 0;
  const counts: Record<string, string> = {
    providers: `${providers.data?.length || 0} 件`,
    models: `${models.data?.length || 0} 件`,
    auto: `${settings.data?.data.auto_model_ids?.length || 0} 件`,
    memory: "記憶を管理",
    tools: `${tools.data?.length || 0} 件`,
    mcp: `${mcp.data?.filter((row) => row.data.enabled).length || 0} 件接続中`,
    browser: "操作と許可先",
    "web-search": "検索の設定",
    general: "表示・既定モデル",
    network: domains ? `${domains} ドメイン` : "制限なし",
    account: "変更・確認",
    backups: "作成・復元",
    usage: "30日間",
    statistics: "30日間",
    diagnostics: "状態を確認",
  };
  const defaultModel = models.data?.find((row) => row.id === settings.data?.data.default_model_id);
  return <div className="settings-overview">
    <Title title={t("設定の概要")} sub={t("AIの接続、使い方、安全性をここから管理できます。")} />
    <section className="settings-hero">
      <div><p className="eyebrow">YOUR WORKSPACE</p><h2>{t("いつでも、あなたの使い方に。")}</h2><p>{t("接続したAIと権限はこの環境だけで管理されます。")}</p></div>
      <Link to="/settings/providers" className="button btn-primary">{t("Providerを追加")} <ArrowRight size={16} /></Link>
    </section>
    <section className="settings-summary" aria-label={t("現在の設定")}>
      <div><span>{t("既定モデル")}</span><b>{defaultModel?.data.name || defaultModel?.data.model_id || t("未設定")}</b></div>
      <div><span>{t("接続状態")}</span><b>{providers.data?.length ? t("準備完了") : t("Providerを追加")}</b></div>
      <div><span>{t("テーマ")}</span><b>{t("表示設定で変更")}</b></div>
    </section>
    {groups.map((group) => <section key={group.label} className="settings-overview-section">
      <h3 className="settings-overview-label">{t(group.label)}</h3>
      <div className="settings-overview-grid">
        {group.cards.map(([path, title, description, Icon]) => <Link key={path} to={"/settings/" + path} className="settings-overview-card">
          <span className="settings-card-icon"><Icon size={20} /></span><span className="settings-card-copy"><b>{t(title)}</b><small>{t(description)}</small></span><strong>{t(counts[path])}</strong><ArrowRight size={17} />
        </Link>)}
      </div>
    </section>)}
  </div>;
}
