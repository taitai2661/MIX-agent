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
  ChevronsLeft,
  Archive,
  Folder,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from "react";
import { NavLink, Route, Routes, useNavigate } from "react-router-dom";

import { Empty, ErrorBox, Logo } from "@/components/shared";
import { ThemeController } from "@/components/theme";
import { Agents } from "@/features/agents/Agents";
import { Auth } from "@/features/auth/Auth";
import { Chat } from "@/features/chat/Chat";
import { Conversations } from "@/features/chat/Conversations";
import { Memories } from "@/features/memory/Memories";
import { Skills } from "@/features/skills/Skills";
import { Settings } from "@/features/settings/Settings";
import { Setup } from "@/features/setup/Setup";

export function App() {
  const sidebarMinimum = 220,
    sidebarMaximum = 380,
    sidebarDefault = 248;
  const [user, setUser] = useState<any>(null),
    [ready, setReady] = useState(false),
    [needsAdmin, setNeedsAdmin] = useState(false),
    [error, setError] = useState<unknown>(null),
    [sidebar, setSidebar] = useState(false),
    [collapsed, setCollapsed] = useState(false),
    [resizingSidebar, setResizingSidebar] = useState(false),
    [sidebarWidth, setSidebarWidth] = useState(() => {
      const stored = Number(localStorage.getItem("mix-agent-sidebar-width"));
      return Number.isFinite(stored) && stored >= sidebarMinimum && stored <= sidebarMaximum ? stored : sidebarDefault;
    });
  const navigate = useNavigate();
  const qc = useQueryClient();
  useEffect(() => {
    localStorage.setItem("mix-agent-sidebar-width", String(sidebarWidth));
  }, [sidebarWidth]);
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
    queryFn: () => api("/conversations?state=active"),
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
  function resizeSidebar(event: PointerEvent<HTMLDivElement>) {
    if (window.matchMedia("(max-width: 760px)").matches) return;
    event.preventDefault();
    const initialX = event.clientX;
    const initialWidth = sidebarWidth;
    setResizingSidebar(true);
    const onMove = (moveEvent: globalThis.PointerEvent) => {
      setSidebarWidth(Math.min(sidebarMaximum, Math.max(sidebarMinimum, initialWidth + moveEvent.clientX - initialX)));
    };
    const onEnd = () => {
      setResizingSidebar(false);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onEnd);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onEnd, { once: true });
  }
  function resizeSidebarWithKeyboard(event: KeyboardEvent<HTMLDivElement>) {
    const increment = event.shiftKey ? 24 : 8;
    if (event.key === "ArrowLeft") setSidebarWidth((width) => Math.max(sidebarMinimum, width - increment));
    else if (event.key === "ArrowRight") setSidebarWidth((width) => Math.min(sidebarMaximum, width + increment));
    else if (event.key === "Home") setSidebarWidth(sidebarMinimum);
    else if (event.key === "End") setSidebarWidth(sidebarMaximum);
    else return;
    event.preventDefault();
  }
  return (
    <div className={"shell " + (sidebar ? "sidebar-open " : "") + (collapsed ? "sidebar-collapsed " : "") + (resizingSidebar ? "sidebar-resizing" : "")}>
      <aside id="main-navigation" className="sidebar" aria-label="メインメニュー" style={{ "--sidebar-width": `${sidebarWidth}px` } as CSSProperties}>
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
        <div className="sidebar-label">ピン留め</div>
        <div className="history">
          {conversations.data?.filter((c) => c.data.pinned).map((c) => (
            <NavLink
              onClick={() => setSidebar(false)}
              key={c.id}
              to={"/chat/" + c.id}
            >
              {c.data.title}
            </NavLink>
          ))}
          {!conversations.data?.some((c) => c.data.pinned) && <small>ピン留めした会話が表示されます</small>}
          <div className="sidebar-label compact"><Folder size={12} />最近のチャット</div>
          {conversations.data?.filter((c) => !c.data.pinned).map((c) => <NavLink onClick={() => setSidebar(false)} key={c.id} to={"/chat/" + c.id}>{c.data.title}</NavLink>)}
          <NavLink className="conversation-manage-link" to="/history"><Archive size={15} />会話を管理</NavLink>
          <NavLink className="conversation-manage-link" to="/history/trash"><Trash2 size={15} />ごみ箱</NavLink>
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
      <div className="sidebar-resizer" role="separator" aria-label="サイドバーの幅を調整" aria-orientation="vertical" aria-valuemin={sidebarMinimum} aria-valuemax={sidebarMaximum} aria-valuenow={sidebarWidth} tabIndex={0} onPointerDown={resizeSidebar} onKeyDown={resizeSidebarWithKeyboard} />
      <div className="main">
        <header className="topbar">
          <button
            className="icon"
            aria-label="メニュー"
            aria-expanded={window.matchMedia("(max-width: 760px)").matches ? sidebar : undefined}
            aria-controls="main-navigation"
            onClick={() => window.matchMedia("(max-width: 760px)").matches ? setSidebar(!sidebar) : setCollapsed(!collapsed)}
          >
            {collapsed ? <PanelLeft size={19} /> : <ChevronsLeft size={19} />}
          </button>
          <span className="topbar-title"><b>MIX agent</b></span>
        </header>
        <Routes>
          <Route path="/" element={<Chat />} />
          <Route path="/chat/:id" element={<Chat />} />
          <Route path="/history" element={<Conversations />} />
          <Route path="/history/:state" element={<Conversations />} />
          <Route path="/agents" element={<Agents />} />
          <Route path="/memory" element={<Memories />} />
          <Route path="/skills" element={<Skills />} />
          <Route path="/setup" element={<Setup />} />
          <Route
            path="/settings/*"
            element={
              <Settings
                user={user}
                onUserChange={(value) => {
                  setCSRF(value.csrf);
                  setUser(value);
                }}
                onLogout={() => {
                  setUser(null);
                  qc.clear();
                }}
              />
            }
          />
          <Route path="*" element={<Empty>ページが見つかりません</Empty>} />
        </Routes>
      </div>
    </div>
  );
}
