import {
  Database,
  Globe,
  Server,
  Settings2,
  ShieldCheck,
  Sparkles,
  BarChart3,
  Brain,
} from "lucide-react";
import { NavLink, Route, Routes } from "react-router-dom";

import { MCP } from "@/features/mcp/MCP";
import { Models } from "@/features/providers/Models";
import { Providers } from "@/features/providers/Providers";
import { Backups } from "@/features/settings/Backups";
import { Account } from "@/features/settings/Account";
import { General } from "@/features/settings/General";
import { Tools } from "@/features/tools/Tools";
import { BrowserSettings, WebSearchSettings } from "@/features/settings/ToolSettings";
import { SettingsOverview } from "@/features/settings/Overview";
import { Statistics } from "@/features/settings/Statistics";
import { Memories } from "@/features/memory/Memories";

export function Settings({
  user,
  onUserChange,
  onLogout,
}: {
  user: { username: string };
  onUserChange: (user: any) => void;
  onLogout: () => void;
}) {
  return (
    <div className="settings-layout">
      <nav className="settings-nav">
        <div className="settings-nav-heading"><h2>設定</h2><p>ワークスペースを管理</p></div>
        <NavLink className="settings-home" to="/settings" end>
          <Settings2 size={17} /> 概要
        </NavLink>
        <p className="settings-nav-label">AI の設定</p>
        {[
          ["providers", "AI Providers", Server],
          ["models", "モデル", Sparkles],
          ["memory", "Memory", Brain],
        ].map(([path, label, Icon]: any) => (
          <NavLink key={path} to={"/settings/" + path}>
            <Icon size={17} />
            {label}
          </NavLink>
        ))}
        <p className="settings-nav-label">接続と安全性</p>
        <NavLink to="/settings/statistics"><BarChart3 size={17} /> 統計</NavLink>
        {[
          ["browser", "Browser", Globe],
          ["web-search", "Web検索", Globe],
          ["tools", "Tools・権限", ShieldCheck],
          ["mcp", "MCP", Globe],
          ["general", "一般・通信", Settings2],
          ["account", "アカウント・安全性", ShieldCheck],
          ["backups", "バックアップ", Database],
        ].map(([path, label, Icon]: any) => (
          <NavLink key={path} to={"/settings/" + path}>
            <Icon size={17} />
            {label}
          </NavLink>
        ))}
      </nav>
      <main className="settings-content">
        <Routes>
          <Route index element={<SettingsOverview />} />
          <Route path="providers" element={<Providers />} />
          <Route path="models" element={<Models />} />
          <Route path="memory" element={<Memories />} />
          <Route path="tools" element={<Tools />} />
          <Route path="browser" element={<BrowserSettings />} />
          <Route path="web-search" element={<WebSearchSettings />} />
          <Route path="mcp" element={<MCP />} />
          <Route path="general" element={<General />} />
          <Route path="account" element={<Account user={user} onUserChange={onUserChange} onLogout={onLogout} />} />
          <Route path="backups" element={<Backups />} />
          <Route path="statistics" element={<Statistics />} />
        </Routes>
      </main>
    </div>
  );
}
