import { api, setCSRF, type Row } from "@/app/api";
import { ja } from "@/app/strings";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Bot,
  BookOpen,
  Brain,
  LogOut,
  MessageSquare,
  PanelLeft,
  Plus,
  Settings2,
  ShieldCheck,
  ChevronsLeft,
  X,
} from "lucide-react";
import { useEffect, useState } from "react";
import { NavLink, Route, Routes, useNavigate } from "react-router-dom";

import { Empty, ErrorBox, Logo } from "@/components/shared";
import { ThemeController } from "@/components/theme";
import { Agents } from "@/features/agents/Agents";
import { Auth } from "@/features/auth/Auth";
import { Chat } from "@/features/chat/Chat";
import { Memories } from "@/features/memory/Memories";
import { Skills } from "@/features/skills/Skills";
import { Settings } from "@/features/settings/Settings";
import { Setup } from "@/features/setup/Setup";

export function App() {
  const [user, setUser] = useState<any>(null),
    [ready, setReady] = useState(false),
    [needsAdmin, setNeedsAdmin] = useState(false),
    [error, setError] = useState<unknown>(null),
    [sidebar, setSidebar] = useState(false),
    [collapsed, setCollapsed] = useState(false);
  const navigate = useNavigate();
  const qc = useQueryClient();
  useEffect(() => {
    Promise.all([api("/setup"), api("/auth/me").catch(() => null)])
      .then(([setup, me]) => {
        setNeedsAdmin(setup.needs_admin);
        if (me) {
          setCSRF(me.csrf);
          setUser(me);
        }
        setReady(true);
      })
      .catch(setError);
  }, []);
  const conversations = useQuery<Row[]>({
    queryKey: ["/conversations"],
    queryFn: () => api("/conversations"),
    enabled: !!user,
  });
  async function authenticated(value: any) {
    setCSRF(value.csrf);
    setUser(value);
    if (needsAdmin) navigate("/setup");
    setNeedsAdmin(false);
  }
  if (error)
    return (
      <main className="auth">
        <ErrorBox error={error} />
        <Button onClick={() => location.reload()}>再読み込み</Button>
      </main>
    );
  if (!ready) return <main className="auth">{ja.loading}</main>;
  if (!user) return <Auth setup={needsAdmin} onDone={authenticated} />;
  async function newChat() {
    navigate("/");
    setSidebar(false);
  }
  return (
    <div className={"shell " + (sidebar ? "sidebar-open " : "") + (collapsed ? "sidebar-collapsed" : "")}>
      <aside className="sidebar" aria-label="メインメニュー">
        <div className="sidebar-brand">
          <Logo />
          <div className="sidebar-controls"><ThemeController /><button className="icon mobile" onClick={() => setSidebar(false)} aria-label="閉じる"><X size={18} /></button></div>
        </div>
        <Button variant="outline" className="new-chat" onClick={newChat}>
          <Plus size={17} />
          {ja.newChat}
          <kbd>＋</kbd>
        </Button>
        <nav className="primary-nav">
          <NavLink to="/" end>
            <MessageSquare size={18} />
            {ja.chat}
          </NavLink>
          <NavLink to="/agents">
            <Bot size={18} />
            {ja.agents}
          </NavLink>
          <NavLink to="/memory">
            <Brain size={18} />
            {ja.memory}
          </NavLink>
          <NavLink to="/skills">
            <BookOpen size={18} />
            {ja.skills}
          </NavLink>
        </nav>
        <div className="sidebar-label">最近のチャット</div>
        <div className="history">
          {conversations.data?.map((c) => (
            <NavLink
              onClick={() => setSidebar(false)}
              key={c.id}
              to={"/chat/" + c.id}
            >
              {c.data.title}
            </NavLink>
          ))}
          {!conversations.data?.length && (
            <small>会話はここに保存されます</small>
          )}
        </div>
        <div className="sidebar-bottom">
          <div className="local-badge">
            <span />
            SELF-HOSTED<span className="version">v0.1</span>
          </div>
          <NavLink to="/settings/providers">
            <Settings2 size={18} />
            {ja.settings}
          </NavLink>
          <div className="profile">
            <span className="avatar">{user.username[0].toUpperCase()}</span>
            <div>
              <b>{user.username}</b>
              <small>Personal workspace</small>
            </div>
            <button
              className="icon"
              aria-label="ログアウト"
              onClick={async () => {
                await api("/auth/logout", "POST");
                setUser(null);
                qc.clear();
              }}
            >
              <LogOut size={16} />
            </button>
          </div>
        </div>
      </aside>
      <div className="main">
        <header className="topbar">
          <button
            className="icon"
            aria-label="メニュー"
            onClick={() => window.matchMedia("(max-width: 760px)").matches ? setSidebar(!sidebar) : setCollapsed(!collapsed)}
          >
            {collapsed ? <PanelLeft size={19} /> : <ChevronsLeft size={19} />}
          </button>
          <span className="topbar-title"><b>MIX agent</b><small>Personal workspace</small></span>
          <span className="private-label">
            <ShieldCheck size={14} /> あなたの環境で、あなたのAIを。
          </span>
        </header>
        <Routes>
          <Route path="/" element={<Chat />} />
          <Route path="/chat/:id" element={<Chat />} />
          <Route path="/agents" element={<Agents />} />
          <Route path="/memory" element={<Memories />} />
          <Route path="/skills" element={<Skills />} />
          <Route path="/setup" element={<Setup />} />
          <Route path="/settings/*" element={<Settings />} />
          <Route path="*" element={<Empty>ページが見つかりません</Empty>} />
        </Routes>
      </div>
    </div>
  );
}
