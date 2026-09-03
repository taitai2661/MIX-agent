import { api, setCSRF, type Row } from "@/app/api";
import { ja } from "@/app/strings";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Bot,
  BookOpen,
  LogOut,
  MessageSquare,
  PanelLeft,
  Plus,
  Settings2,
  Clock3,
  ChevronsLeft,
  Archive,
  Folder,
  Trash2,
  X,
  MoreVertical,
  Pencil,
  Pin,
} from "lucide-react";
import { useEffect, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from "react";
import { Navigate, NavLink, Route, Routes, useNavigate } from "react-router-dom";

import { Empty, ErrorBox, Logo } from "@/components/shared";
import { ThemeController } from "@/components/theme";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import { ConfirmModal, PromptModal } from "@/components/modal";
import { Agents } from "@/features/agents/Agents";
import { Auth } from "@/features/auth/Auth";
import { Chat } from "@/features/chat/Chat";
import { Conversations } from "@/features/chat/Conversations";
import { Skills } from "@/features/skills/Skills";
import { Settings } from "@/features/settings/Settings";
import { Setup } from "@/features/setup/Setup";
import { Schedules } from "@/features/schedules/Schedules";

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
  const [openConversationMenu, setOpenConversationMenu] = useState<string | null>(null);
  const [renameTarget, setRenameTarget] = useState<Row | null>(null);
  const [trashTarget, setTrashTarget] = useState<Row | null>(null);
  const folders = useQuery<Row[]>({
    queryKey: ["/conversation-folders"],
    queryFn: () => api("/conversation-folders"),
    enabled: !!user,
  });
  useEffect(() => {
    localStorage.setItem("mix-agent-sidebar-width", String(sidebarWidth));
  }, [sidebarWidth]);
  useEffect(() => {
    if (!openConversationMenu) return;
    const close = (event: MouseEvent) => {
      if (!(event.target as HTMLElement).closest(".conversation-sidebar-item")) setOpenConversationMenu(null);
    };
    const keydown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") setOpenConversationMenu(null);
    };
    document.addEventListener("click", close);
    document.addEventListener("keydown", keydown);
    return () => { document.removeEventListener("click", close); document.removeEventListener("keydown", keydown); };
  }, [openConversationMenu]);
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
  async function updateConversation(id: string, data: object) {
    await api(`/conversations/${id}/state`, "PATCH", data);
    setOpenConversationMenu(null);
    await Promise.all([
      qc.invalidateQueries({ queryKey: ["/conversations"] }),
      qc.invalidateQueries({ queryKey: ["/conversation-folders"] }),
    ]);
  }
  async function renameConversation(conversation: Row) {
    setRenameTarget(conversation);
  }
  async function trashConversation(conversation: Row) {
    setTrashTarget(conversation);
  }
  function conversationItem(conversation: Row) {
    const menuOpen = openConversationMenu === conversation.id;
    return <div className="conversation-sidebar-item" key={conversation.id}>
      <NavLink onClick={() => setSidebar(false)} className="conversation-sidebar-link" to={"/chat/" + conversation.id}>
        {conversation.data.title}
      </NavLink>
      <button className="conversation-sidebar-menu-button" aria-label={`${conversation.data.title}のメニュー`} aria-expanded={menuOpen} onClick={(event) => { event.preventDefault(); event.stopPropagation(); setOpenConversationMenu(menuOpen ? null : conversation.id); }}><MoreVertical size={17} /></button>
      {menuOpen && <div className="conversation-sidebar-menu" role="menu">
        <button role="menuitem" onClick={() => updateConversation(conversation.id, { pinned: !conversation.data.pinned })}><Pin size={17} />{conversation.data.pinned ? "ピン留めを外す" : "ピン留め"}<kbd>P</kbd></button>
        <button role="menuitem" onClick={() => renameConversation(conversation)}><Pencil size={17} />名前を変更<kbd>R</kbd></button>
        <div className="conversation-sidebar-menu-group"><Folder size={17} /><span>グループに移動</span><select aria-label="グループに移動" value={conversation.data.folder_id || ""} onChange={(event) => updateConversation(conversation.id, { folder_id: event.target.value || null })}><option value="">未分類</option>{folders.data?.map((folder) => <option key={folder.id} value={folder.id}>{folder.data.name}</option>)}</select></div>
        <button role="menuitem" onClick={() => updateConversation(conversation.id, { archived: true })}><Archive size={17} />アーカイブ</button>
        <button className="danger" role="menuitem" onClick={() => trashConversation(conversation)}><Trash2 size={17} />削除<kbd>D</kbd></button>
      </div>}
    </div>;
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
        <nav className="primary-nav">
          <div className="chat-nav-item">
            <NavLink to="/" end onClick={() => setSidebar(false)}>
              <MessageSquare size={18} />
              {ja.chat}
            </NavLink>
            <button className="chat-nav-new" onClick={newChat} aria-label={ja.newChat} title={ja.newChat}>
              <Plus size={17} />
            </button>
          </div>
          <NavLink to="/agents" onClick={() => setSidebar(false)}>
            <Bot size={18} />
            {ja.agents}
          </NavLink>
          <NavLink to="/skills" onClick={() => setSidebar(false)}>
            <BookOpen size={18} />
            {ja.skills}
          </NavLink>
          <NavLink to="/schedules" onClick={() => setSidebar(false)}><Clock3 size={18} />定期実行</NavLink>
        </nav>
        <div className="sidebar-label">ピン留め</div>
        <div className="history">
          {conversations.data?.filter((c) => c.data.pinned).map(conversationItem)}
          {!conversations.data?.some((c) => c.data.pinned) && <small>ピン留めした会話が表示されます</small>}
          <div className="sidebar-label compact"><Folder size={12} />最近のチャット</div>
          {conversations.data?.filter((c) => !c.data.pinned).map(conversationItem)}
          <NavLink className="conversation-manage-link" to="/history" onClick={() => setSidebar(false)}><Archive size={15} />会話を管理</NavLink>
          <NavLink className="conversation-manage-link" to="/history/trash" onClick={() => setSidebar(false)}><Trash2 size={15} />ごみ箱</NavLink>
        </div>
        <div className="sidebar-bottom">
          <div className="local-badge">
            <span />
            SELF-HOSTED<span className="version">v0.1</span>
          </div>
          <NavLink to="/settings/providers" onClick={() => setSidebar(false)}>
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
      {sidebar && <button className="sidebar-backdrop" type="button" aria-label="メニューを閉じる" onClick={() => setSidebar(false)} />}
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
            {sidebar ? <X size={19} /> : collapsed ? <PanelLeft size={19} /> : <ChevronsLeft size={19} />}
          </button>
          <span className="topbar-title"><b>MIX agent</b></span>
        </header>
        <ErrorBoundary>
          <Routes>
            <Route path="/" element={<Chat />} />
            <Route path="/chat/:id" element={<Chat />} />
            <Route path="/history" element={<Conversations />} />
            <Route path="/history/:state" element={<Conversations />} />
            <Route path="/agents" element={<Agents />} />
            <Route path="/memory" element={<Navigate to="/settings/memory" replace />} />
            <Route path="/skills" element={<Skills />} />
            <Route path="/schedules" element={<Schedules />} />
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
        </ErrorBoundary>
        {renameTarget && <PromptModal
          title="チャット名を変更"
          defaultValue={renameTarget.data.title}
          onConfirm={(title) => {
            setRenameTarget(null);
            if (!title.trim() || title.trim() === renameTarget.data.title) return;
            void (async () => {
              await api(`/conversations/${renameTarget.id}`, "PATCH", { title: title.trim() });
              setOpenConversationMenu(null);
              await qc.invalidateQueries({ queryKey: ["/conversations"] });
            })();
          }}
          onCancel={() => setRenameTarget(null)}
        />}
        {trashTarget && <ConfirmModal
          title="チャットを削除"
          message="このチャットをごみ箱へ移動しますか？"
          onConfirm={() => {
            const id = trashTarget.id;
            setTrashTarget(null);
            void (async () => {
              await api(`/conversations/${id}`, "DELETE");
              setOpenConversationMenu(null);
              await qc.invalidateQueries({ queryKey: ["/conversations"] });
            })();
          }}
          onCancel={() => setTrashTarget(null)}
        />}
      </div>
    </div>
  );
}
