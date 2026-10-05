import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { api, listenRunEvents } from "@/lib/api";
import { useRunEvents } from "@/lib/runStore";
import { CONVERSATIONS_QUERY_KEY, PENDING_APPROVALS_QUERY_KEY } from "@/lib/queries";
import { useUi, type SettingsTab, type View } from "@/lib/ui";
import type { UpdateNotice } from "@/lib/types";

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
      const { mode, modelId, activeProjectId } = useUi.getState();
      return api.createConversation({
        mode,
        model_id: modelId,
        title: null,
        workspace_id: null,
        project_id: activeProjectId,
      });
    },
    onSuccess: (created) => {
      void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
      setConversationId(created.id);
      setView("chat");
    },
  });
}

/** The update banner's state, plus the two ways it gets set. */
export function useUpdateNotice() {
  const [update, setUpdate] = useState<UpdateNotice | null>(null);

  const check = useCallback(async () => {
    try {
      setUpdate(await api.checkForUpdates());
    } catch {
      setUpdate(null);
    }
  }, []);

  useEffect(() => {
    let active = true;
    const offs: Array<() => void> = [];
    listen<UpdateNotice>("update_available", (e) => {
      if (active) setUpdate(e.payload);
    }).then((un) => (active ? offs.push(un) : un()));
    listen("menu://check-updates", () => {
      if (active) void check();
    }).then((un) => (active ? offs.push(un) : un()));
    return () => {
      active = false;
      offs.forEach((un) => un());
    };
  }, [check]);

  return { update, dismiss: () => setUpdate(null), check };
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
        if (evt.kind === "tool_approval_requested" || evt.kind === "tool_approval_resolved") {
          void queryClient.invalidateQueries({ queryKey: PENDING_APPROVALS_QUERY_KEY });
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
    // DOM-level event so the command palette can trigger the same flow.
    const onNewThread = () => {
      if (active) handlersRef.current.newThread();
    };
    window.addEventListener("mix:new-thread", onNewThread);
    track(
      listen("scheduled_runs_started", () => {
        if (active) void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
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
  const setSurface = useUi((s) => s.setSurface);
  const setShortcutsOpen = useUi((s) => s.setShortcutsOpen);
  const setters = useRef({ setPaletteOpen, toggleFileTree, setSurface, setShortcutsOpen });
  useEffect(() => {
    setters.current = { setPaletteOpen, toggleFileTree, setSurface, setShortcutsOpen };
  });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const {
        setPaletteOpen: palette,
        toggleFileTree: tree,
        setSurface: surface,
        setShortcutsOpen: shortcuts,
      } = setters.current;
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        palette(true);
      }
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "e") {
        e.preventDefault();
        tree();
      }
      // Surface shortcuts: ⌘⇧C → chat, ⌘⇧D → desktop-agent.
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && !e.altKey) {
        const key = e.key.toLowerCase();
        if (key === "c") {
          e.preventDefault();
          surface("chat");
        } else if (key === "d") {
          e.preventDefault();
          surface("desktop-agent");
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
