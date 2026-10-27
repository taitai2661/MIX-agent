import { useEffect, useRef } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { sendNotification } from "@tauri-apps/plugin-notification";
import { api, listenRunEvents } from "@/lib/api";
import { useRunEvents } from "@/lib/runStore";
import {
  CONVERSATIONS_QUERY_KEY,
  PENDING_APPROVALS_QUERY_KEY,
} from "@/lib/queries";
import { useUi, type SettingsTab, type View } from "@/lib/ui";
import { t } from "@/lib/i18n";

const ZOOM_KEY = "mix-agent-zoom";
function applyZoom(next: number) {
  const value = Math.min(1.5, Math.max(0.8, Math.round(next * 10) / 10));
  try {
    localStorage.setItem(ZOOM_KEY, String(value));
  } catch {
    // Storage can be unavailable in a preview; zoom still applies for the session.
  }
  document.documentElement.style.zoom = String(value);
}
function currentZoom() {
  try {
    const stored = Number(localStorage.getItem(ZOOM_KEY));
    return Number.isFinite(stored) && stored > 0 ? stored : 1;
  } catch {
    return 1;
  }
}

/** Open a blank chat without creating an empty conversation row.
 *
 * The backend conversation is created on first send by `ChatPane`, so clicking
 * 新規チャット three times no longer leaves three untitled rows behind.
 */
export function useNewThread() {
  const setConversationId = useUi((s) => s.setConversationId);
  const setView = useUi((s) => s.setView);
  return () => {
    setConversationId(null);
    setView("chat");
  };
}

/** Create a thread from whatever mode / model / project is selected *now*.
 *
 * Shared by the header, the sidebar and the menu so the three paths cannot
 * drift — they used to each build their own mutation with their own closure,
 * which meant a menu-triggered thread could inherit a stale mode.
 */
export function useCreateThread() {
  const queryClient = useQueryClient();
  const setConversationId = useUi((s) => s.setConversationId);
  const setView = useUi((s) => s.setView);
  return useMutation({
    mutationFn: () => {
      const { surface, mode, modelId, activeProjectId } = useUi.getState();
      const isAgent = surface === "desktop-agent";
      const threadMode = isAgent
        ? "agent"
        : mode === "thinking"
          ? "thinking"
          : "chat";
      const projectId = isAgent ? activeProjectId : null;
      return api.createConversation({
        mode: threadMode,
        model_id: modelId,
        title: null,
        workspace_id: null,
        project_id: projectId,
      });
    },
    onSuccess: (created) => {
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
      setConversationId(created.id);
      setView("chat");
    },
  });
}

/** Tauri menu events, the run-event bridge and cache invalidations.
 *
 * Split out of `App` so the component stops owning eight listeners; the hook
 * takes callbacks instead of reading the store itself, which keeps it
 * independent of what a menu item happens to target. */
export function useShellEvents(handlers: {
  newThread: () => void;
  openView: (view: View) => void;
  openSettingsTab: (tab: SettingsTab) => void;
}) {
  const pushEvent = useRunEvents((s) => s.push);
  const queryClient = useQueryClient();

  // Keep the latest callbacks without re-subscribing on every render.
  // Written in an effect rather than during render: a render that React throws
  // away must not be able to leave a ref pointing at handlers from a tree that
  // was never committed.
  const handlersRef = useRef(handlers);
  useEffect(() => {
    handlersRef.current = handlers;
  });

  useEffect(() => {
    let active = true;
    const offs: Array<() => void> = [];
    const track = (p: Promise<() => void>) => {
      p.then((un) => (active ? offs.push(un) : un()));
    };

    track(
      listenRunEvents((evt) => {
        if (!active) return;
        pushEvent(evt);
        // Approval cards are pushed as run events, so the pending list can be
        // invalidated on the signal that actually changes it rather than only
        // re-read on a timer.
        if (
          evt.kind === "tool_approval_requested" ||
          evt.kind === "tool_approval_resolved"
        ) {
          void queryClient.invalidateQueries({
            queryKey: PENDING_APPROVALS_QUERY_KEY,
          });
        }
      }),
    );

    track(
      listen("menu://settings", () => {
        if (active) handlersRef.current.openView("settings");
      }),
    );
    const MENU_SETTINGS_TABS: Record<string, SettingsTab> = {
      "menu://backup": "backup",
      "menu://diagnostics": "diagnostics",
    };
    for (const [event, tab] of Object.entries(MENU_SETTINGS_TABS)) {
      track(
        listen(event, () => {
          if (active) handlersRef.current.openSettingsTab(tab);
        }),
      );
    }
    track(
      listen("menu://new-chat", () => {
        if (!active) return;
        handlersRef.current.openView("chat");
        window.dispatchEvent(new CustomEvent("mix:new-thread"));
      }),
    );
    track(
      listen("app://hidden-to-tray", () => {
        if (!active) return;
        try {
          sendNotification({
            title: "MIX-agent",
            body: t("実行はバックグラウンドで継続しています。トレイから再表示できます。"),
          });
        } catch {
          // Notification permission may be denied; the tray icon is still there.
        }
      }),
    );
    // DOM-level event so the command palette can trigger the same flow.
    const onNewThread = () => {
      if (active) handlersRef.current.newThread();
    };
    window.addEventListener("mix:new-thread", onNewThread);
    track(
      listen("scheduled_runs_started", () => {
        if (active)
          void queryClient.invalidateQueries({
            queryKey: CONVERSATIONS_QUERY_KEY,
          });
      }),
    );
    track(
      listen("mcp_oauth_complete", () => {
        if (active) void queryClient.invalidateQueries({ queryKey: ["mcp"] });
      }),
    );

    return () => {
      active = false;
      offs.forEach((un) => un());
      window.removeEventListener("mix:new-thread", onNewThread);
    };
  }, [pushEvent, queryClient]);
}

/** Global keyboard shortcuts. */
export function useAppShortcuts() {
  const setPaletteOpen = useUi((s) => s.setPaletteOpen);
  const toggleFileTree = useUi((s) => s.toggleFileTree);
  const toggleSidebar = useUi((s) => s.toggleSidebar);
  const openSettings = useUi((s) => s.openSettings);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const setSurface = useUi((s) => s.setSurface);
  const setShortcutsOpen = useUi((s) => s.setShortcutsOpen);
  const setters = useRef({
    setPaletteOpen,
    toggleFileTree,
    toggleSidebar,
    openSettings,
    setRightPanel,
    setSurface,
    setShortcutsOpen,
  });
  useEffect(() => {
    setters.current = {
      setPaletteOpen,
      toggleFileTree,
      toggleSidebar,
      openSettings,
      setRightPanel,
      setSurface,
      setShortcutsOpen,
    };
  });

  useEffect(() => {
    applyZoom(currentZoom());
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const {
        setPaletteOpen: palette,
        toggleFileTree: tree,
        toggleSidebar: sidebar,
        setRightPanel: rightPanelSetter,
        setSurface: surface,
        setShortcutsOpen: shortcuts,
      } = setters.current;

      const isMod = e.metaKey || e.ctrlKey;

      // Text scaling for low-vision users / small laptop panels. WebView zoom
      // is not exposed by the platform menu, so keep the standard chords here.
      if (isMod && !e.altKey && (e.key === "=" || e.key === "+")) {
        e.preventDefault();
        applyZoom(currentZoom() + 0.1);
        return;
      }
      if (isMod && !e.altKey && e.key === "-") {
        e.preventDefault();
        applyZoom(currentZoom() - 0.1);
        return;
      }
      if (isMod && !e.altKey && e.key === "0") {
        e.preventDefault();
        applyZoom(1);
        return;
      }

      if (isMod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        palette(true);
        return;
      }
      if (isMod && !e.shiftKey && e.key.toLowerCase() === "b") {
        e.preventDefault();
        sidebar();
        return;
      }
      if (isMod && e.shiftKey && e.key.toLowerCase() === "e") {
        e.preventDefault();
        tree();
        return;
      }
      // Surface shortcuts: ⌘⇧C → chat, ⌘⇧D → desktop-agent.
      if (isMod && e.shiftKey && !e.altKey) {
        const key = e.key.toLowerCase();
        if (key === "c") {
          e.preventDefault();
          surface("chat");
          return;
        } else if (key === "d") {
          e.preventDefault();
          surface("desktop-agent");
          return;
        }
      }
      // Esc: Close right panel if open and not in an input
      if (e.key === "Escape" && !isMod && !e.shiftKey && !e.altKey) {
        const state = useUi.getState();
        if (
          state.rightPanel !== "none" &&
          !state.paletteOpen &&
          !state.shortcutsOpen
        ) {
          e.preventDefault();
          rightPanelSetter("none");
          return;
        }
      }
      // `?` (Shift+/) — only when the user is not typing into a field, so a
      // question mark can still be typed into a prompt. `e.target` is not
      // always an Element (a key on the window/document is possible), and only
      // Elements have `closest`.
      if (e.key === "?" && !e.metaKey && !e.ctrlKey && !e.altKey) {
        const target = e.target;
        if (
          target instanceof Element &&
          target.closest("input, textarea, select, [contenteditable='true']")
        ) {
          return;
        }
        e.preventDefault();
        shortcuts(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
