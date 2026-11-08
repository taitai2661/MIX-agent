import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Archive,
  BookOpen,
  Bot,
  Clock3,
  Folder,
  LogOut,
  MessageSquare,
  MoreVertical,
  PanelLeftClose,
  Pencil,
  Pin,
  Plus,
  Search,
  Settings2,
  Trash2,
  X,
} from "lucide-react";
import {
  Fragment,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type Dispatch,
  type KeyboardEvent,
  type PointerEvent,
  type RefObject,
  type SetStateAction,
} from "react";
import { NavLink, useNavigate } from "react-router-dom";

import { api, type Row } from "@/app/api";
import { t } from "@/app/i18n";
import { ja } from "@/app/strings";
import { ConfirmModal, PromptModal } from "@/components/modal";
import { Logo } from "@/components/shared";
import { ThemeController } from "@/components/theme";
import { useToast } from "@/components/toast";
import {
  HISTORY_GROUPS,
  historyGroup,
  historyTime,
  type HistoryGroup,
} from "@/lib/format";
import { SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH } from "./sidebarConfig";

export interface AppSidebarProps {
  user: { username: string };
  /** Rows for the active conversation list; the shell owns the query so the
   *  welcome card and the sidebar read the same cache entry. */
  rows: Row[];
  /** More active conversations exist beyond the loaded pages. */
  hasMoreConversations: boolean;
  loadingMoreConversations: boolean;
  onLoadMoreConversations: () => void;
  isMobile: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  collapsed: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
  width: number;
  onWidthChange: Dispatch<SetStateAction<number>>;
  onResizingChange: (resizing: boolean) => void;
  /** Focus returns here when the mobile drawer closes. */
  toggleRef: RefObject<HTMLButtonElement | null>;
  onLoggedOut: () => void;
}

export function AppSidebar(props: AppSidebarProps) {
  const {
    user,
    rows,
    hasMoreConversations,
    loadingMoreConversations,
    onLoadMoreConversations,
    isMobile,
    open,
    onOpenChange,
    collapsed,
    onCollapsedChange,
    width,
    onWidthChange,
    onResizingChange,
    toggleRef,
    onLoggedOut,
  } = props;
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { addToast } = useToast();
  const sidebarRef = useRef<HTMLElement>(null);
  const sidebarCloseRef = useRef<HTMLButtonElement>(null);
  const conversationMenuRef = useRef<HTMLDivElement>(null);
  const conversationMenuTriggerRef = useRef<HTMLButtonElement | null>(null);
  const returnMenuFocusRef = useRef(false);
  const [openConversationMenu, setOpenConversationMenu] = useState<
    string | null
  >(null);
  const [conversationQuery, setConversationQuery] = useState("");
  const [folderFilter, setFolderFilter] = useState("");
  const [renameTarget, setRenameTarget] = useState<Row | null>(null);
  const [trashTarget, setTrashTarget] = useState<Row | null>(null);
  const folders = useQuery<Row[]>({
    queryKey: ["/conversation-folders"],
    queryFn: ({ signal }) =>
      api("/conversation-folders", "GET", undefined, undefined, { signal }),
  });

  const updateConversation = useMemo(
    () => async (id: string, data: object) => {
      await api(`/conversations/${id}/state`, "PATCH", data);
      returnMenuFocusRef.current = true;
      setOpenConversationMenu(null);
      await Promise.all([
        qc.invalidateQueries({ queryKey: ["/conversations"] }),
        qc.invalidateQueries({ queryKey: ["/conversation-folders"] }),
      ]);
    },
    [qc],
  );

  const filteredConversations = useMemo(() => {
    const query = conversationQuery.trim().toLocaleLowerCase();
    return rows.filter((conversation) => {
      if (
        folderFilter &&
        String(conversation.data.folder_id || "") !== folderFilter
      ) {
        return false;
      }
      if (!query) return true;
      return String(conversation.data.title || "")
        .toLocaleLowerCase()
        .includes(query);
    });
  }, [conversationQuery, folderFilter, rows]);

  const pinnedConversations = useMemo(
    () =>
      filteredConversations.filter((conversation) => conversation.data.pinned),
    [filteredConversations],
  );

  const groupedConversations = useMemo(() => {
    const groups: Record<HistoryGroup, Row[]> = {
      today: [],
      yesterday: [],
      week: [],
      older: [],
    };
    filteredConversations
      .filter((conversation) => !conversation.data.pinned)
      .forEach((conversation) => {
        groups[
          historyGroup(
            conversation.data.last_message_at || conversation.created_at,
          )
        ].push(conversation);
      });
    return groups;
  }, [filteredConversations]);

  const folderName = useMemo(
    () => (id?: string | null) =>
      folders.data?.find((folder) => folder.id === id)?.data.name as
        | string
        | undefined,
    [folders.data],
  );

  useEffect(() => {
    if (!openConversationMenu) return;
    const close = (event: MouseEvent) => {
      if (
        !(event.target as HTMLElement).closest(".conversation-sidebar-item")
      ) {
        setOpenConversationMenu(null);
      }
    };
    const keydown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        returnMenuFocusRef.current = true;
        setOpenConversationMenu(null);
      }
      const target = rows.find((row) => row.id === openConversationMenu);
      if (!target) return;
      if (
        event.target instanceof HTMLElement &&
        /^(INPUT|SELECT|TEXTAREA)$/.test(event.target.tagName)
      ) {
        return;
      }
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
    return () => {
      document.removeEventListener("click", close);
      document.removeEventListener("keydown", keydown);
    };
  }, [openConversationMenu, rows, updateConversation]);

  useEffect(() => {
    if (!openConversationMenu) return;
    conversationMenuRef.current
      ?.querySelector<HTMLElement>('[role="menuitem"]')
      ?.focus();
  }, [openConversationMenu]);

  useEffect(() => {
    if (openConversationMenu || !returnMenuFocusRef.current) return;
    returnMenuFocusRef.current = false;
    conversationMenuTriggerRef.current?.focus();
  }, [openConversationMenu]);

  useEffect(() => {
    const el = sidebarRef.current;
    if (!el) return;
    if (isMobile && !open) {
      el.setAttribute("inert", "");
      el.setAttribute("aria-hidden", "true");
    } else {
      el.removeAttribute("inert");
      el.removeAttribute("aria-hidden");
    }
  }, [isMobile, open]);

  useEffect(() => {
    if (!isMobile || !open) return;
    const el = sidebarRef.current;
    const returnFocus = document.activeElement as HTMLElement | null;
    const sidebarToggle = toggleRef.current;
    sidebarCloseRef.current?.focus();
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onOpenChange(false);
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
      (returnFocus ?? sidebarToggle)?.focus();
    };
  }, [isMobile, onOpenChange, open, toggleRef]);

  async function newChat() {
    void navigate("/");
    onOpenChange(false);
  }

  function conversationItem(conversation: Row) {
    const menuOpen = openConversationMenu === conversation.id;
    return (
      <div className="conversation-sidebar-item" key={conversation.id}>
        <NavLink
          onClick={() => onOpenChange(false)}
          className="conversation-sidebar-link"
          to={"/chat/" + conversation.id}
          title={conversation.data.title}
        >
          <span className="conversation-sidebar-title">
            {conversation.data.title}
          </span>
          <span className="conversation-sidebar-meta">
            <span>
              {historyTime(
                conversation.data.last_message_at || conversation.created_at,
              )}
            </span>
            {folderName(conversation.data.folder_id) && (
              <>
                <span aria-hidden="true">·</span>
                <span>{folderName(conversation.data.folder_id)}</span>
              </>
            )}
          </span>
        </NavLink>
        <button
          className="conversation-sidebar-menu-button"
          aria-label={t("{title}のメニュー", {
            title: conversation.data.title,
          })}
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            conversationMenuTriggerRef.current = event.currentTarget;
            setOpenConversationMenu(menuOpen ? null : conversation.id);
          }}
        >
          <MoreVertical size={17} />
        </button>
        {menuOpen && (
          <div
            ref={conversationMenuRef}
            className="conversation-sidebar-menu"
            role="menu"
            aria-label={t("会話の操作")}
            tabIndex={-1}
            onKeyDown={(event) => {
              const items = Array.from(
                event.currentTarget.querySelectorAll<HTMLElement>(
                  '[role="menuitem"]',
                ),
              );
              if (!items.length) return;
              const index = items.indexOf(
                document.activeElement as HTMLElement,
              );
              if (event.key === "ArrowDown") {
                event.preventDefault();
                items[(index + 1) % items.length]?.focus();
              } else if (event.key === "ArrowUp") {
                event.preventDefault();
                items[index <= 0 ? items.length - 1 : index - 1]?.focus();
              } else if (event.key === "Home") {
                event.preventDefault();
                items[0]?.focus();
              } else if (event.key === "End") {
                event.preventDefault();
                items[items.length - 1]?.focus();
              } else if (event.key === "Escape") {
                event.preventDefault();
                returnMenuFocusRef.current = true;
                setOpenConversationMenu(null);
              }
            }}
          >
            <button
              role="menuitem"
              onClick={() =>
                updateConversation(conversation.id, {
                  pinned: !conversation.data.pinned,
                })
              }
            >
              <Pin size={17} />
              {t(conversation.data.pinned ? "ピン留めを外す" : "ピン留め")}
              <kbd>P</kbd>
            </button>
            <button
              role="menuitem"
              onClick={() => {
                setRenameTarget(conversation);
                setOpenConversationMenu(null);
              }}
            >
              <Pencil size={17} />
              {t("名前を変更")}
              <kbd>R</kbd>
            </button>
            <div className="conversation-sidebar-menu-group">
              <Folder size={17} />
              <span>{t("グループに移動")}</span>
              <select
                aria-label={t("グループに移動")}
                value={conversation.data.folder_id || ""}
                onChange={(event) =>
                  updateConversation(conversation.id, {
                    folder_id: event.target.value || null,
                  })
                }
              >
                <option value="">{t("未分類")}</option>
                {folders.data?.map((folder) => (
                  <option key={folder.id} value={folder.id}>
                    {folder.data.name}
                  </option>
                ))}
              </select>
            </div>
            <button
              role="menuitem"
              onClick={() =>
                updateConversation(conversation.id, { archived: true })
              }
            >
              <Archive size={17} />
              {t("アーカイブ")}
            </button>
            <button
              className="danger"
              role="menuitem"
              onClick={() => {
                setTrashTarget(conversation);
                setOpenConversationMenu(null);
              }}
            >
              <Trash2 size={17} />
              {t("削除")}
              <kbd>D</kbd>
            </button>
          </div>
        )}
      </div>
    );
  }

  function resizeSidebar(event: PointerEvent<HTMLDivElement>) {
    if (window.matchMedia("(max-width: 760px)").matches) return;
    event.preventDefault();
    const initialX = event.clientX;
    const initialWidth = width;
    onResizingChange(true);
    const onMove = (moveEvent: globalThis.PointerEvent) => {
      onWidthChange(
        Math.min(
          SIDEBAR_MAX_WIDTH,
          Math.max(
            SIDEBAR_MIN_WIDTH,
            initialWidth + moveEvent.clientX - initialX,
          ),
        ),
      );
    };
    const onEnd = () => {
      onResizingChange(false);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onEnd);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onEnd, { once: true });
  }

  function resizeSidebarWithKeyboard(event: KeyboardEvent<HTMLDivElement>) {
    const increment = event.shiftKey ? 24 : 8;
    if (event.key === "ArrowLeft") {
      onWidthChange((value) => Math.max(SIDEBAR_MIN_WIDTH, value - increment));
    } else if (event.key === "ArrowRight") {
      onWidthChange((value) => Math.min(SIDEBAR_MAX_WIDTH, value + increment));
    } else if (event.key === "Home") {
      onWidthChange(SIDEBAR_MIN_WIDTH);
    } else if (event.key === "End") {
      onWidthChange(SIDEBAR_MAX_WIDTH);
    } else {
      return;
    }
    event.preventDefault();
  }

  async function logout() {
    try {
      await api("/auth/logout", "POST");
    } catch {
      // Local dismissal still signs out the UI; the revoked session cookie
      // makes any subsequent request 401.
    }
    void qc.cancelQueries();
    qc.clear();
    onLoggedOut();
  }

  return (
    <>
      <aside
        id="main-navigation"
        ref={sidebarRef}
        className={"sidebar" + (openConversationMenu ? " menu-open" : "")}
        aria-label={t("メインメニュー")}
        style={{ "--sidebar-width": `${width}px` } as CSSProperties}
      >
        <div className="sidebar-brand">
          {collapsed && !isMobile ? (
            <button
              className="brand-toggle"
              type="button"
              onClick={() => onCollapsedChange(false)}
              aria-label={t("サイドバーを展開する")}
              title={t("サイドバーを展開する")}
            >
              <Logo />
            </button>
          ) : (
            <Logo />
          )}
          <div className="sidebar-controls">
            <div className="sidebar-theme">
              <ThemeController />
            </div>
            <button
              className="icon sidebar-collapse-toggle"
              onClick={() => onCollapsedChange(true)}
              aria-label={t("サイドバーを折りたたむ")}
              title={t("サイドバーを折りたたむ")}
            >
              <PanelLeftClose size={18} />
            </button>
            <button
              className="icon mobile"
              ref={sidebarCloseRef}
              onClick={() => onOpenChange(false)}
              aria-label={t("閉じる")}
            >
              <X size={18} />
            </button>
          </div>
        </div>
        <nav className="primary-nav" aria-label={t("ワークスペース")}>
          <div className="sidebar-label nav-label">{t("ワークスペース")}</div>
          <div className="chat-nav-item">
            <NavLink
              to="/"
              end
              onClick={() => onOpenChange(false)}
              aria-label={t(ja.chat)}
              data-tooltip={t(ja.chat)}
            >
              <MessageSquare size={18} />
              {t(ja.chat)}
            </NavLink>
            <button
              className="chat-nav-new"
              onClick={newChat}
              aria-label={t(ja.newChat)}
              data-tooltip={t(ja.newChat)}
            >
              <Plus size={17} />
            </button>
          </div>
          <NavLink
            to="/agents"
            onClick={() => onOpenChange(false)}
            aria-label={t(ja.agents)}
            data-tooltip={t(ja.agents)}
          >
            <Bot size={18} />
            {t(ja.agents)}
          </NavLink>
          <NavLink
            to="/projects"
            onClick={() => onOpenChange(false)}
            aria-label={t("プロジェクト")}
            data-tooltip={t("プロジェクト")}
          >
            <Folder size={18} />
            {t("プロジェクト")}
          </NavLink>
          <NavLink
            to="/skills"
            onClick={() => onOpenChange(false)}
            aria-label={t(ja.skills)}
            data-tooltip={t(ja.skills)}
          >
            <BookOpen size={18} />
            {t(ja.skills)}
          </NavLink>
          <NavLink
            to="/schedules"
            onClick={() => onOpenChange(false)}
            aria-label={t("定期実行")}
            data-tooltip={t("定期実行")}
          >
            <Clock3 size={18} />
            {t("定期実行")}
          </NavLink>
        </nav>
        <div className="sidebar-search">
          <Search size={14} aria-hidden="true" />
          <input
            value={conversationQuery}
            onChange={(event) => setConversationQuery(event.target.value)}
            placeholder={t("会話を検索")}
            aria-label={t("会話を検索")}
          />
          {conversationQuery && (
            <button
              type="button"
              className="icon"
              onClick={() => setConversationQuery("")}
              aria-label={t("検索をクリア")}
              title={t("検索をクリア")}
            >
              <X size={13} />
            </button>
          )}
        </div>
        {folders.data?.length ? (
          <div className="sidebar-folders" aria-label={t("フォルダ")}>
            <button
              type="button"
              className={folderFilter ? "" : "active"}
              aria-pressed={!folderFilter}
              onClick={() => setFolderFilter("")}
            >
              {t("すべて")}
            </button>
            {folders.data.map((folder) => (
              <button
                key={folder.id}
                type="button"
                className={folderFilter === folder.id ? "active" : ""}
                aria-pressed={folderFilter === folder.id}
                onClick={() =>
                  setFolderFilter((current) =>
                    current === folder.id ? "" : folder.id,
                  )
                }
              >
                {folder.data.name}
              </button>
            ))}
          </div>
        ) : null}
        {pinnedConversations.length > 0 && (
          <div className="sidebar-label">{t("ピン留め")}</div>
        )}
        <div className="history">
          {pinnedConversations.map(conversationItem)}
          {HISTORY_GROUPS.map((group) => {
            const items = groupedConversations[group];
            if (!items.length) return null;
            const label = {
              today: t("今日"),
              yesterday: t("昨日"),
              week: t("過去7日"),
              older: t("それ以前"),
            }[group];
            return (
              <Fragment key={group}>
                <div className="sidebar-label compact">{label}</div>
                {items.map(conversationItem)}
              </Fragment>
            );
          })}
          {!filteredConversations.length && !hasMoreConversations && (
            <div className="sidebar-empty">
              {t("該当する会話はありません。")}
            </div>
          )}
          {hasMoreConversations && (
            <button
              type="button"
              className="conversation-manage-link conversation-load-more"
              disabled={loadingMoreConversations}
              onClick={onLoadMoreConversations}
            >
              {loadingMoreConversations
                ? t("読み込み中…")
                : t("以前の会話を読み込む")}
            </button>
          )}
          <NavLink
            className="conversation-manage-link"
            to="/history"
            onClick={() => onOpenChange(false)}
          >
            <Archive size={15} />
            {t("会話を管理")}
          </NavLink>
          <NavLink
            className="conversation-manage-link"
            to="/history/trash"
            onClick={() => onOpenChange(false)}
          >
            <Trash2 size={15} />
            {t("ごみ箱")}
          </NavLink>
        </div>
        <div className="sidebar-bottom">
          <div className="local-badge">
            <span />
            SELF-HOSTED
            <span className="version">v{__APP_VERSION__}</span>
          </div>
          <NavLink
            to="/settings/providers"
            onClick={() => onOpenChange(false)}
            aria-label={t(ja.settings)}
            data-tooltip={t(ja.settings)}
          >
            <Settings2 size={18} />
            {t(ja.settings)}
          </NavLink>
          <div className="profile">
            <span className="avatar" data-tooltip={user.username}>
              {user.username[0].toUpperCase()}
            </span>
            <div>
              <b>{user.username}</b>
            </div>
            <button
              className="icon"
              aria-label={t(ja.logout)}
              data-tooltip={t(ja.logout)}
              onClick={() => void logout()}
            >
              <LogOut size={16} />
            </button>
          </div>
        </div>
      </aside>
      {open && (
        <button
          className="sidebar-backdrop"
          type="button"
          aria-label={t("メニューを閉じる")}
          onClick={() => onOpenChange(false)}
        />
      )}
      {/* Focusable window splitter (ARIA APG): separator is interactive when
          it controls a resize. */}
      {/* eslint-disable jsx-a11y/no-noninteractive-element-interactions, jsx-a11y/no-noninteractive-tabindex -- focusable separator is a splitter widget */}
      <div
        className="sidebar-resizer"
        role="separator"
        aria-label={t("サイドバーの幅を調整")}
        aria-orientation="vertical"
        aria-valuemin={SIDEBAR_MIN_WIDTH}
        aria-valuemax={SIDEBAR_MAX_WIDTH}
        aria-valuenow={width}
        tabIndex={0}
        onPointerDown={resizeSidebar}
        onKeyDown={resizeSidebarWithKeyboard}
      />
      {/* eslint-enable jsx-a11y/no-noninteractive-element-interactions, jsx-a11y/no-noninteractive-tabindex */}
      {renameTarget && (
        <PromptModal
          title={t("チャット名を変更")}
          defaultValue={renameTarget.data.title}
          onConfirm={(title) => {
            setRenameTarget(null);
            if (!title.trim() || title.trim() === renameTarget.data.title)
              return;
            void (async () => {
              try {
                await api(`/conversations/${renameTarget.id}`, "PATCH", {
                  title: title.trim(),
                });
                setOpenConversationMenu(null);
                await qc.invalidateQueries({ queryKey: ["/conversations"] });
                addToast({ type: "success", title: t("名前を変更しました") });
              } catch (cause) {
                addToast({
                  type: "error",
                  title: t("操作に失敗しました"),
                  message:
                    cause instanceof Error ? cause.message : String(cause),
                });
              }
            })();
          }}
          onCancel={() => setRenameTarget(null)}
        />
      )}
      {trashTarget && (
        <ConfirmModal
          title={t("チャットを削除")}
          message={t("このチャットをごみ箱へ移動しますか？")}
          confirmLabel={t("ごみ箱へ移動")}
          tone="danger"
          onConfirm={() => {
            const id = trashTarget.id;
            setTrashTarget(null);
            void (async () => {
              try {
                await api(`/conversations/${id}`, "DELETE");
                setOpenConversationMenu(null);
                await qc.invalidateQueries({ queryKey: ["/conversations"] });
                addToast({ type: "success", title: t("ごみ箱へ移動しました") });
              } catch (cause) {
                addToast({
                  type: "error",
                  title: t("操作に失敗しました"),
                  message:
                    cause instanceof Error ? cause.message : String(cause),
                });
              }
            })();
          }}
          onCancel={() => setTrashTarget(null)}
        />
      )}
    </>
  );
}
