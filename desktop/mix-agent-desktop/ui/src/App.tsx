import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { FolderTree, GitBranch, PanelRight, Sparkles } from "lucide-react";
import { api, listenRunEvents } from "./lib/api";
import { cn } from "./lib/cn";
import { useRunEvents } from "./lib/runStore";
import { useUi, type SettingsTab, type View } from "./lib/ui";
import { t } from "./lib/i18n";
import { normalizeTheme, storedTheme } from "./lib/theme";

import { ChatPane } from "./features/chat/ChatPane";
import { AgentsPane } from "./features/agents/AgentsPane";
import { ProjectsPane } from "./features/projects/ProjectsPane";
import { SchedulesPane } from "./features/schedules/SchedulesPane";
import { SettingsPane } from "./features/settings/SettingsPane";
import { FileTreePane } from "./features/files/FileTreePane";
import { Sidebar } from "./features/shell/Sidebar";
import { RightPanel } from "./features/shell/RightPanel";
import { CommandPalette } from "./features/shell/CommandPalette";
import { Button } from "./components/ui/button";

const VIEW_TITLES: Record<View, string> = {
  chat: "チャット",
  agents: "エージェント",
  projects: "プロジェクト",
  schedules: "スケジュール",
  settings: "設定",
};

export function App() {
  const view = useUi((s) => s.view);
  const setView = useUi((s) => s.setView);
  const locale = useUi((s) => s.locale);
  const sidebarOpen = useUi((s) => s.sidebarOpen);
  const toggleSidebar = useUi((s) => s.toggleSidebar);
  const rightPanel = useUi((s) => s.rightPanel);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const fileTreeOpen = useUi((s) => s.fileTreeOpen);
  const toggleFileTree = useUi((s) => s.toggleFileTree);
  const setPaletteOpen = useUi((s) => s.setPaletteOpen);
  const openSettings = useUi((s) => s.openSettings);
  const setTheme = useUi((s) => s.setTheme);
  const setLocale = useUi((s) => s.setLocale);
  const conversationId = useUi((s) => s.conversationId);
  const queryClient = useQueryClient();
  const health = useQuery({ queryKey: ["health"], queryFn: api.health, refetchInterval: 15000 });

  // Sync persisted settings (language/theme) once at boot.
  const settings = useQuery({ queryKey: ["settings"], queryFn: api.getSettings, staleTime: Infinity });
  useEffect(() => {
    const data = settings.data;
    if (!data) return;
    if (data.language === "ja" || data.language === "en") {
      if (data.language !== locale) setLocale(data.language);
    }
    const theme = normalizeTheme(data.theme);
    if (theme !== storedTheme()) setTheme(theme);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.data]);

  const [update, setUpdate] = useState<{
    version: string; current_version: string; notes: string; date: string | null;
  } | null>(null);

  const mode = useUi((s) => s.mode);
  const modelId = useUi((s) => s.modelId);
  const activeProjectId = useUi((s) => s.activeProjectId);
  const setConversationId = useUi((s) => s.setConversationId);
  const createThread = useMutation({
    mutationFn: () =>
      api.createConversation({
        mode, model_id: modelId, title: null, workspace_id: null, project_id: activeProjectId,
      }),
    onSuccess: (created) => {
      void queryClient.invalidateQueries({ queryKey: ["conversations"] });
      setConversationId(created.id);
      setView("chat");
    },
  });

  // All run events flow through the throttled live-run store.
  const pushEvent = useRunEvents((s) => s.push);
  useEffect(() => {
    let active = true;
    const offs: Array<() => void> = [];

    listenRunEvents((evt) => {
      if (active) pushEvent(evt);
    }).then((un) => { if (active) offs.push(un); else un(); });

    const MENU_ACTIONS: Record<string, View> = {
      "menu://settings": "settings",
    };
    for (const [event, target] of Object.entries(MENU_ACTIONS)) {
      listen(event, () => { if (active) setView(target); }).then(
        (un) => { if (active) offs.push(un); else un(); },
      );
    }
    // Menu items that target a specific Settings tab rather than a view.
    const MENU_SETTINGS_TABS: Record<string, SettingsTab> = {
      "menu://backup": "backup",
      "menu://diagnostics": "diagnostics",
    };
    for (const [event, tab] of Object.entries(MENU_SETTINGS_TABS)) {
      listen(event, () => { if (active) openSettings(tab); }).then(
        (un) => { if (active) offs.push(un); else un(); },
      );
    }
    listen("menu://new-chat", () => {
      if (!active) return;
      setView("chat");
      window.dispatchEvent(new CustomEvent("mix:new-thread"));
    }).then((un) => { if (active) offs.push(un); else un(); });
    // DOM-level event so the command palette can trigger the same flow.
    const onNewThread = () => {
      if (active) createThread.mutate();
    };
    window.addEventListener("mix:new-thread", onNewThread);
    listen("menu://check-updates", async () => {
      if (!active) return;
      try {
        setUpdate(await api.checkForUpdates());
      } catch {
        setUpdate(null);
      }
    }).then((un) => { if (active) offs.push(un); else un(); });
    listen<{
      version: string; current_version: string; notes: string; date: string | null;
    }>("update_available", (e) => {
      if (active) setUpdate(e.payload);
    }).then((un) => { if (active) offs.push(un); else un(); });
    listen("scheduled_runs_started", () => {
      if (active) queryClient.invalidateQueries({ queryKey: ["conversations"] });
    }).then((un) => { if (active) offs.push(un); else un(); });
    listen("mcp_oauth_complete", () => {
      if (active) queryClient.invalidateQueries({ queryKey: ["mcp"] });
    }).then((un) => { if (active) offs.push(un); else un(); });

    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen(true);
      }
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "e") {
        e.preventDefault();
        toggleFileTree();
      }
    };
    window.addEventListener("keydown", onKey);

    return () => {
      active = false;
      offs.forEach((un) => un());
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mix:new-thread", onNewThread);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const conversations = useQuery({ queryKey: ["conversations"], queryFn: api.listConversations });
  const projects = useQuery({
    queryKey: ["projects"], queryFn: api.listProjects, staleTime: 30_000,
  });
  const activeProject = projects.data?.find((p) => p.id === activeProjectId) ?? null;
  const threadTitle = useMemo(
    () => conversations.data?.find((c) => c.id === conversationId)?.title,
    [conversations.data, conversationId],
  );

  return (
    <div className="flex h-full min-w-0">
      <Sidebar />
      <section className="flex min-w-0 flex-1 flex-col border-l border-line bg-bg">
        <header
          className="flex h-11 shrink-0 items-center gap-2 border-b border-line px-3"
          data-tauri-drag-region
        >
          <Button
            variant="ghost"
            size="icon"
            onClick={toggleSidebar}
            title="Toggle sidebar"
            aria-label="Toggle sidebar"
          >
            <PanelRight size={15} className={sidebarOpen ? "-scale-x-100" : ""} />
          </Button>
          <span className="truncate text-sm font-medium text-fg">
            {view === "chat" ? (threadTitle ?? t("チャット")) : t(VIEW_TITLES[view])}
          </span>
          {view === "chat" && activeProject && (
            <span
              className="truncate text-xs text-muted"
              title={activeProject.rootPath ?? undefined}
            >
              {activeProject.name}
            </span>
          )}
          <span className="flex-1" />
          <span
            className={cn(
              "size-2 shrink-0 rounded-full",
              health.data?.status === "ok"
                ? "bg-success"
                : health.isError
                  ? "bg-danger"
                  : "bg-warning",
            )}
            title={`Health: ${health.data?.status ?? "…"}`}
          />
          {view === "chat" && (
            <Button
              variant="ghost"
              size="icon"
              data-active={fileTreeOpen}
              onClick={toggleFileTree}
              title={`${t("ファイルツリー")} (⌘⇧E)`}
              aria-label={t("ファイルツリー")}
              aria-pressed={fileTreeOpen}
            >
              <FolderTree size={15} />
            </Button>
          )}
          {view === "chat" && (
            <Button
              variant="ghost"
              size="icon"
              data-active={rightPanel === "changes"}
              onClick={() => setRightPanel(rightPanel === "changes" ? "activity" : "changes")}
              title={t("変更")}
              aria-label={t("変更")}
              aria-pressed={rightPanel === "changes"}
            >
              <GitBranch size={15} />
            </Button>
          )}
          {view === "chat" && (
            <Button
              variant="ghost"
              size="icon"
              data-active={rightPanel !== "none"}
              onClick={() => setRightPanel(rightPanel === "none" ? "activity" : "none")}
              title={t("このRunのアクティビティ")}
              aria-label={t("このRunのアクティビティ")}
              aria-pressed={rightPanel !== "none"}
            >
              <PanelRight size={15} />
            </Button>
          )}
        </header>

        {update && (
          <div className="flex shrink-0 items-center gap-3 border-b border-line bg-brand-soft px-3 py-2 text-xs text-brand">
            <Sparkles size={14} className="shrink-0" />
            <span className="flex-1">
              Update available: {update.current_version} → <strong>{update.version}</strong>
            </span>
            <Button
              variant="primary"
              size="sm"
              onClick={() => void api.installUpdate()}
            >
              Install &amp; restart
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setUpdate(null)}>
              Dismiss
            </Button>
          </div>
        )}

        <div className="flex min-h-0 flex-1 flex-row">
          {view === "chat" && fileTreeOpen && <FileTreePane />}
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            {view === "chat" && <ChatPane />}
            {view === "agents" && <AgentsPane />}
            {view === "projects" && <ProjectsPane />}
            {view === "schedules" && <SchedulesPane />}
            {view === "settings" && <SettingsPane />}
          </div>
        </div>
      </section>
      {view === "chat" && <RightPanel />}
      <CommandPalette />
    </div>
  );
}
