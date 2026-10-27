import { api, setCSRF, type Row } from "@/app/api";
import { ja } from "@/app/strings";
import { getLanguage, setLanguage, t, useLanguage } from "@/app/i18n";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Bot,
  BookOpen,
  LogOut,
  MessageSquare,
  PanelLeft,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Settings2,
  Clock3,
  Archive,
  Folder,
  Trash2,
  X,
  MoreVertical,
  Pencil,
  Pin,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent } from "react";
import { Navigate, NavLink, Route, Routes, useLocation, useNavigate, useParams } from "react-router-dom";

import { Empty, ErrorBox, Logo } from "@/components/shared";
import { ThemeController } from "@/components/theme";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import { ConfirmModal, PromptModal } from "@/components/modal";
import { ToastProvider } from "@/components/toast";
import { Tour } from "@/components/tour";
import { Agents } from "@/features/agents/Agents";
import { Auth } from "@/features/auth/Auth";
import { Chat } from "@/features/chat/Chat";
import { Conversations } from "@/features/chat/Conversations";
import type { WorkspaceSnapshot } from "@/features/chat/greeting";
import { lazy, Suspense } from "react";

const Skills = lazy(() => import("@/features/skills/Skills").then(m => ({ default: m.Skills })));
const Settings = lazy(() => import("@/features/settings/Settings").then(m => ({ default: m.Settings })));
const Setup = lazy(() => import("@/features/setup/Setup").then(m => ({ default: m.Setup })));
const Schedules = lazy(() => import("@/features/schedules/Schedules").then(m => ({ default: m.Schedules })));
const Projects = lazy(() => import("@/features/projects/Projects").then(m => ({ default: m.Projects })));

function useMediaQuery(query: string) {
  const [matches, setMatches] = useState(
    () => typeof window !== "undefined" && window.matchMedia(query).matches,
  );
  useEffect(() => {
    const list = window.matchMedia(query);
    const update = () => setMatches(list.matches);
    update();
    list.addEventListener("change", update);
    return () => list.removeEventListener("change", update);
  }, [query]);
  return matches;
}

type Me = { username: string; csrf: string };

/** Keyed wrapper so switching conversations remounts the composer. That lets
 *  the per-conversation draft survive a session-expiry round trip through the
 *  login screen instead of vanishing with the unmounted component. */
function ChatRoute({ workspace }: { workspace: WorkspaceSnapshot | null }) {
  const { id } = useParams();
  return <Chat key={id ?? "new"} workspace={workspace} />;
}

export function App() {
  useLanguage();
  const sidebarMinimum = 220,
    sidebarMaximum = 380,
    sidebarDefault = 248;
  const [user, setUser] = useState<Me | null>(null),
    [ready, setReady] = useState(false),
    [needsAdmin, setNeedsAdmin] = useState(false),
    [error, setError] = useState<unknown>(null),
    [sessionExpired, setSessionExpired] = useState(false),
    [sidebar, setSidebar] = useState(false),
    [collapsed, setCollapsed] = useState(() => localStorage.getItem("mix-agent-sidebar-collapsed") === "1"),
    [resizingSidebar, setResizingSidebar] = useState(false),
    [sidebarWidth, setSidebarWidth] = useState(() => {
      const stored = Number(localStorage.getItem("mix-agent-sidebar-width"));
      return Number.isFinite(stored) && stored >= sidebarMinimum && stored <= sidebarMaximum ? stored : sidebarDefault;
    });
  const isMobile = useMediaQuery("(max-width: 760px)");
  const navigate = useNavigate();
  const location = useLocation();
  const firstRoute = useRef(true);
  const sidebarRef = useRef<HTMLElement>(null);
  const sidebarToggleRef = useRef<HTMLButtonElement>(null);
  const sidebarCloseRef = useRef<HTMLButtonElement>(null);
  const qc = useQueryClient();
  const [openConversationMenu, setOpenConversationMenu] = useState<string | null>(null);
  const [renameTarget, setRenameTarget] = useState<Row | null>(null);
  const [trashTarget, setTrashTarget] = useState<Row | null>(null);
  const [showTour, setShowTour] = useState(false);
  const folders = useQuery<Row[]>({
    queryKey: ["/conversation-folders"],
    queryFn: () => api("/conversation-folders"),
    enabled: !!user,
  });
  useEffect(() => {
    localStorage.setItem("mix-agent-sidebar-width", String(sidebarWidth));
  }, [sidebarWidth]);
  useEffect(() => {
    localStorage.setItem("mix-agent-sidebar-collapsed", collapsed ? "1" : "0");
  }, [collapsed]);
  useEffect(() => {
    if (!openConversationMenu) return;
    const close = (event: MouseEvent) => {
      if (!(event.target as HTMLElement).closest(".conversation-sidebar-item")) setOpenConversationMenu(null);
    };
    const keydown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") setOpenConversationMenu(null);
      const target = conversations.data?.find((row) => row.id === openConversationMenu);
      if (!target) return;
      if (event.target instanceof HTMLElement && /^(INPUT|SELECT|TEXTAREA)$/.test(event.target.tagName)) return;
      const key = event.key.toLowerCase();
      if (key === "p") {
        event.preventDefault();
        void updateConversation(target.id, { pinned: !target.data.pinned });
      } else if (key === "r") {
        event.preventDefault();
        setRenameTarget(target);
        setOpenConversationMenu(null);
      } else if (key === "d") {
        event.preventDefault();
        setTrashTarget(target);
        setOpenConversationMenu(null);
      }
    };
    document.addEventListener("click", close);
    document.addEventListener("keydown", keydown);
    return () => { document.removeEventListener("click", close); document.removeEventListener("keydown", keydown); };
  }, [openConversationMenu]);
  useEffect(() => {
    Promise.all([api("/setup"), api("/auth/me").catch(() => null)])
      .then(async ([setup, me]) => {
        setNeedsAdmin(setup.needs_admin);
        if (me) {
          setCSRF(me.csrf);
          const settings = await api("/settings");
          setLanguage(settings.data.ui_language === "en" ? "en" : "ja");
          setUser(me);
        }
        setReady(true);
        // 初回ログイン時にツアーを表示
        if (me && !localStorage.getItem("mix-agent-tour-done")) {
          setShowTour(true);
        }
      })
      .catch(setError);
  }, []);
  const conversations = useQuery<Row[]>({
    queryKey: ["/conversations"],
    queryFn: () => api("/conversations?state=active"),
    enabled: !!user,
  });
  useEffect(() => {
    // A session can expire while the tab is open.  The API client emits this
    // event instead of surfacing a generic network error, so drop the cached
    // identity and let the Auth screen take over without a full reload.
    const unauthorized = () => {
      setCSRF("");
      setSessionExpired(true);
      setUser(null);
    };
    window.addEventListener("mix:unauthorized", unauthorized);
    return () => window.removeEventListener("mix:unauthorized", unauthorized);
  }, []);
  useEffect(() => {
    const path = location.pathname;
    const label = path === "/" ? "新しいチャット"
      : path.startsWith("/chat/") ? "チャット"
      : path.startsWith("/projects") ? "プロジェクト"
      : path.startsWith("/history") ? "会話"
      : path.startsWith("/agents") ? "エージェント"
      : path.startsWith("/skills") ? "スキル"
      : path.startsWith("/schedules") ? "定期実行"
      : path.startsWith("/settings") ? "設定"
      : path.startsWith("/setup") ? "セットアップ"
      : "MIX agent";
    // A conversation owns its own title (set by Chat from the last turn).
    if (!path.startsWith("/chat/")) {
      document.title = label === "MIX agent" ? label : `${t(label)} · MIX agent`;
    }
    if (firstRoute.current) {
      firstRoute.current = false;
      return;
    }
    // Announce the new page to keyboard/screen-reader users. Every route had
    // the same document title and left focus wherever the previous page was.
    document.getElementById("main-content")?.focus({ preventScroll: true });
  }, [location.pathname]);
  useEffect(() => {
    const el = sidebarRef.current;
    if (!el) return;
    if (isMobile && !sidebar) {
      el.setAttribute("inert", "");
      el.setAttribute("aria-hidden", "true");
    } else {
      el.removeAttribute("inert");
      el.removeAttribute("aria-hidden");
    }
  }, [isMobile, sidebar]);
  useEffect(() => {
    if (!isMobile || !sidebar) return;
    const el = sidebarRef.current;
    const returnFocus = document.activeElement as HTMLElement | null;
    sidebarCloseRef.current?.focus();
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setSidebar(false);
        return;
      }
      if (event.key !== "Tab" || !el) return;
      const items = Array.from(
        el.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((node) => node.offsetParent !== null);
      if (!items.length) return;
      const first = items[0];
      const last = items[items.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      (returnFocus ?? sidebarToggleRef.current)?.focus();
    };
  }, [isMobile, sidebar]);
  // 新しいチャット画面の挨拶に使う、ワークスペースの状況だけを渡す。
  const workspace = useMemo<WorkspaceSnapshot | null>(() => {
    const rows = conversations.data;
    if (!rows) return null;
    const stamps = rows
      .map((row) => Date.parse(String(row.data.last_message_at || row.created_at)))
      .filter((stamp) => Number.isFinite(stamp));
    return {
      conversationCount: rows.length,
      pinnedCount: rows.filter((row) => row.data.pinned).length,
      projectCount: 0,
      agentCount: 0,
      lastMessageAt: stamps.length ? Math.max(...stamps) : null,
    };
  }, [conversations.data]);
  async function authenticated(value: Me) {
    setCSRF(value.csrf);
    setSessionExpired(false);
    setUser(value);
    if (needsAdmin) {
      try { await api("/settings", "PUT", { ui_language: getLanguage() }); }
      catch (cause) { setError(cause); return; }
      navigate("/setup");
    } else {
      try { const settings = await api("/settings"); setLanguage(settings.data.ui_language === "en" ? "en" : "ja"); }
      catch (cause) { setError(cause); return; }
    }
    setNeedsAdmin(false);
  }
  if (error)
    return (
      <main className="auth">
        <ErrorBox error={error} />
        <Button onClick={() => window.location.reload()}>{t("再読み込み")}</Button>
      </main>
    );
  if (!ready) return <main className="auth">{t(ja.loading)}</main>;
  if (!user) return <Auth setup={needsAdmin} sessionExpired={sessionExpired} onDone={authenticated} />;
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
      <button className="conversation-sidebar-menu-button" aria-label={`${conversation.data.title} ${t("のメニュー")}`} aria-expanded={menuOpen} onClick={(event) => { event.preventDefault(); event.stopPropagation(); setOpenConversationMenu(menuOpen ? null : conversation.id); }}><MoreVertical size={17} /></button>
      {menuOpen && <div className="conversation-sidebar-menu" role="menu">
        <button role="menuitem" onClick={() => updateConversation(conversation.id, { pinned: !conversation.data.pinned })}><Pin size={17} />{t(conversation.data.pinned ? "ピン留めを外す" : "ピン留め")}<kbd>P</kbd></button>
        <button role="menuitem" onClick={() => renameConversation(conversation)}><Pencil size={17} />{t("名前を変更")}<kbd>R</kbd></button>
        <div className="conversation-sidebar-menu-group"><Folder size={17} /><span>{t("グループに移動")}</span><select aria-label={t("グループに移動")} value={conversation.data.folder_id || ""} onChange={(event) => updateConversation(conversation.id, { folder_id: event.target.value || null })}><option value="">{t("未分類")}</option>{folders.data?.map((folder) => <option key={folder.id} value={folder.id}>{folder.data.name}</option>)}</select></div>
        <button role="menuitem" onClick={() => updateConversation(conversation.id, { archived: true })}><Archive size={17} />{t("アーカイブ")}</button>
        <button className="danger" role="menuitem" onClick={() => trashConversation(conversation)}><Trash2 size={17} />{t("削除")}<kbd>D</kbd></button>
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
    <ToastProvider>
    <div className={"shell " + (sidebar ? "sidebar-open " : "") + (collapsed ? "sidebar-collapsed " : "") + (resizingSidebar ? "sidebar-resizing" : "")}>
      <a className="skip-link" href="#main-content">{t("本文へスキップ")}</a>
      <aside id="main-navigation" ref={sidebarRef} className={"sidebar" + (openConversationMenu ? " menu-open" : "")} aria-label={t("メインメニュー")} style={{ "--sidebar-width": `${sidebarWidth}px` } as CSSProperties}>
        <div className="sidebar-brand">
          {collapsed && !isMobile ? (
            <button className="brand-toggle" type="button" onClick={() => setCollapsed(false)} aria-label={t("サイドバーを展開する")} title={t("サイドバーを展開する")}>
              <Logo />
            </button>
          ) : (
            <Logo />
          )}
          <div className="sidebar-controls">
            <ThemeController />
            <button className="icon sidebar-collapse-toggle" onClick={() => setCollapsed(true)} aria-label={t("サイドバーを折りたたむ")} title={t("サイドバーを折りたたむ")}><PanelLeftClose size={18} /></button>
            <button className="icon mobile" ref={sidebarCloseRef} onClick={() => setSidebar(false)} aria-label={t("閉じる")}><X size={18} /></button>
          </div>
        </div>
        <nav className="primary-nav" aria-label={t("ワークスペース")}>
          <div className="sidebar-label nav-label">{t("ワークスペース")}</div>
          <div className="chat-nav-item">
            <NavLink to="/" end onClick={() => setSidebar(false)} aria-label={t(ja.chat)} data-tooltip={t(ja.chat)}>
              <MessageSquare size={18} />
              {t(ja.chat)}
            </NavLink>
            <button className="chat-nav-new" onClick={newChat} aria-label={t(ja.newChat)} data-tooltip={t(ja.newChat)}>
              <Plus size={17} />
            </button>
          </div>
          <NavLink to="/agents" onClick={() => setSidebar(false)} aria-label={t(ja.agents)} data-tooltip={t(ja.agents)}>
            <Bot size={18} />
            {t(ja.agents)}
          </NavLink>
          <NavLink to="/projects" onClick={() => setSidebar(false)} aria-label={t("プロジェクト")} data-tooltip={t("プロジェクト")}><Folder size={18} />{t("プロジェクト")}</NavLink>
          <NavLink to="/skills" onClick={() => setSidebar(false)} aria-label={t(ja.skills)} data-tooltip={t(ja.skills)}>
            <BookOpen size={18} />
            {t(ja.skills)}
          </NavLink>
          <NavLink to="/schedules" onClick={() => setSidebar(false)} aria-label={t("定期実行")} data-tooltip={t("定期実行")}><Clock3 size={18} />{t("定期実行")}</NavLink>
        </nav>
        {conversations.data?.some((c) => c.data.pinned) && <div className="sidebar-label">{t("ピン留め")}</div>}
        <div className="history">
          {conversations.data?.filter((c) => c.data.pinned).map(conversationItem)}
          <div className="sidebar-label compact"><Folder size={12} />{t("最近のチャット")}</div>
          {conversations.data?.filter((c) => !c.data.pinned).map(conversationItem)}
          <NavLink className="conversation-manage-link" to="/history" onClick={() => setSidebar(false)}><Archive size={15} />{t("会話を管理")}</NavLink>
          <NavLink className="conversation-manage-link" to="/history/trash" onClick={() => setSidebar(false)}><Trash2 size={15} />{t("ごみ箱")}</NavLink>
        </div>
        <div className="sidebar-bottom">
          <div className="local-badge">
            <span />
            SELF-HOSTED<span className="version">v0.1</span>
          </div>
          <NavLink to="/settings/providers" onClick={() => setSidebar(false)} aria-label={t(ja.settings)} data-tooltip={t(ja.settings)}>
            <Settings2 size={18} />
            {t(ja.settings)}
          </NavLink>
          <div className="profile">
            <span className="avatar" data-tooltip={user.username}>{user.username[0].toUpperCase()}</span>
            <div>
              <b>{user.username}</b>
            </div>
            <button
              className="icon"
              aria-label={t(ja.logout)}
              data-tooltip={t(ja.logout)}
              onClick={async () => {
                try {
                  await api("/auth/logout", "POST");
                } catch {
                  // Local dismissal still signs out the UI; the revoked
                  // session cookie makes any subsequent request 401.
                }
                qc.cancelQueries();
                qc.clear();
                setUser(null);
              }}
            >
              <LogOut size={16} />
            </button>
          </div>
        </div>
      </aside>
      {sidebar && <button className="sidebar-backdrop" type="button" aria-label={t("メニューを閉じる")} onClick={() => setSidebar(false)} />}
      <div className="sidebar-resizer" role="separator" aria-label={t("サイドバーの幅を調整")} aria-orientation="vertical" aria-valuemin={sidebarMinimum} aria-valuemax={sidebarMaximum} aria-valuenow={sidebarWidth} tabIndex={0} onPointerDown={resizeSidebar} onKeyDown={resizeSidebarWithKeyboard} />
      <main id="main-content" className="main" tabIndex={-1}>
        <header className="topbar">
          {isMobile || collapsed ? (
            <>
              <button
                className="icon"
                ref={sidebarToggleRef}
                aria-label={isMobile ? (sidebar ? t("メニューを閉じる") : t("メニュー")) : t("サイドバーを展開する")}
                aria-expanded={isMobile ? sidebar : undefined}
                aria-controls="main-navigation"
                onClick={() => (isMobile ? setSidebar(!sidebar) : setCollapsed(!collapsed))}
              >
                {isMobile ? (sidebar ? <X size={19} /> : <PanelLeft size={19} />) : <PanelLeftOpen size={19} />}
              </button>
              <span className="topbar-title"><b>MIX agent</b></span>
            </>
          ) : null}
        </header>
        <ErrorBoundary>
          <Suspense fallback={<div className="page-loading">{t("読み込み中…")}</div>}>
          <Routes>
            <Route path="/" element={<ChatRoute workspace={workspace} />} />
            <Route path="/chat/:id" element={<ChatRoute workspace={workspace} />} />
            <Route path="/projects/:projectId/chat" element={<Chat workspace={workspace} />} />
            <Route path="/projects" element={<Projects />} />
            <Route path="/projects/:id" element={<Projects />} />
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
            <Route path="*" element={<Empty>{t("ページが見つかりません")}</Empty>} />
          </Routes>
          </Suspense>
        </ErrorBoundary>
        {renameTarget && <PromptModal
          title={t("チャット名を変更")}
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
          title={t("チャットを削除")}
          message={t("このチャットをごみ箱へ移動しますか？")}
          confirmLabel={t("ごみ箱へ移動")}
          tone="danger"
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
        {showTour && <Tour onClose={() => { setShowTour(false); localStorage.setItem("mix-agent-tour-done", "1"); }} />}
      </main>
    </div>
    </ToastProvider>
  );
}
