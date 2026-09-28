import {
  Activity,
  BarChart3,
  Brain,
  Coins,
  Database,
  Globe,
  Network,
  Server,
  Settings2,
  ShieldCheck,
  Sparkles,
} from "lucide-react";
import { Fragment } from "react";
import { NavLink, Route, Routes } from "react-router-dom";

import { MCP } from "@/features/mcp/MCP";
import { Models } from "@/features/providers/Models";
import { Providers } from "@/features/providers/Providers";
import { Backups } from "@/features/settings/Backups";
import { Account } from "@/features/settings/Account";
import { General } from "@/features/settings/General";
import { Auto } from "@/features/settings/Auto";
import { Tools } from "@/features/tools/Tools";
import { BrowserSettings, WebSearchSettings } from "@/features/settings/ToolSettings";
import { SettingsOverview } from "@/features/settings/Overview";
import { Statistics } from "@/features/settings/Statistics";
import { Usage } from "@/features/settings/Usage";
import { Diagnostics } from "@/features/settings/Diagnostics";
import { NetworkSettings } from "@/features/settings/Network";
import { Memories } from "@/features/memory/Memories";
import { t, useLanguage } from "@/app/i18n";

const groups = [
  { label: null, items: [["", "概要", Settings2]] },
  {
    label: "AI",
    items: [
      ["providers", "AI Providers", Server],
      ["models", "モデル", Sparkles],
      ["auto", "Auto", Sparkles],
      ["memory", "Associative Memory", Brain],
    ],
  },
  {
    label: "ツールと連携",
    items: [
      ["tools", "Tools・権限", ShieldCheck],
      ["mcp", "MCP", Globe],
      ["browser", "Browser", Globe],
      ["web-search", "Web検索", Globe],
    ],
  },
  { label: "基本設定", items: [["general", "一般", Settings2]] },
  {
    label: "セキュリティとデータ",
    items: [
      ["network", "通信とネットワーク", Network],
      ["account", "アカウント・安全性", ShieldCheck],
      ["backups", "バックアップ", Database],
    ],
  },
  {
    label: "運用",
    items: [
      ["usage", "使用量", Coins],
      ["statistics", "統計", BarChart3],
      ["diagnostics", "障害診断", Activity],
    ],
  },
] as const;

export function Settings({
  user,
  onUserChange,
  onLogout,
}: {
  user: { username: string };
  onUserChange: (user: any) => void;
  onLogout: () => void;
}) {
  useLanguage();
  return (
    <div className="settings-layout">
      <nav className="settings-nav">
        <div className="settings-nav-heading"><h2>{t("設定")}</h2><p>{t("ワークスペースを管理")}</p></div>
        {groups.map((group) => (
          <Fragment key={group.label ?? "home"}>
            {group.label && <p className="settings-nav-label">{t(group.label)}</p>}
            {group.items.map(([path, label, Icon]) => (
              <NavLink
                key={path || "overview"}
                to={"/settings" + (path ? "/" + path : "")}
                end={!path}
                className={path ? undefined : "settings-home"}
              >
                <Icon size={17} />
                {t(label)}
              </NavLink>
            ))}
          </Fragment>
        ))}
      </nav>
      <main className="settings-content">
        <Routes>
          <Route index element={<SettingsOverview />} />
          <Route path="providers" element={<Providers />} />
          <Route path="models" element={<Models />} />
          <Route path="auto" element={<Auto />} />
          <Route path="memory" element={<Memories />} />
          <Route path="tools" element={<Tools />} />
          <Route path="browser" element={<BrowserSettings />} />
          <Route path="web-search" element={<WebSearchSettings />} />
          <Route path="network" element={<NetworkSettings />} />
          <Route path="mcp" element={<MCP />} />
          <Route path="general" element={<General />} />
          <Route path="account" element={<Account user={user} onUserChange={onUserChange} onLogout={onLogout} />} />
          <Route path="backups" element={<Backups />} />
          <Route path="usage" element={<Usage />} />
          <Route path="statistics" element={<Statistics />} />
          <Route path="diagnostics" element={<Diagnostics />} />
        </Routes>
      </main>
    </div>
  );
}
