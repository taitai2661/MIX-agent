import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import type { BrowserTabView } from "@/lib/types";
import {
  clearBrowserSession,
  readBrowserSession,
  saveBrowserSession,
} from "./sessionSnapshot";

export interface BrowserPageState {
  url: string | null;
  title: string | null;
  canGoBack: boolean;
  canGoForward: boolean;
}

const EMPTY_PAGE: BrowserPageState = {
  url: null,
  title: null,
  canGoBack: false,
  canGoForward: false,
};

interface BrowserInvokePayload {
  active?: {
    id?: string;
    url?: string | null;
    title?: string | null;
  };
  pages?: BrowserTabView[];
  can_go_back?: boolean;
  can_go_forward?: boolean;
}

/**
 * UI controller for the native child WebView.
 *
 * The page itself is rendered by Tauri, not React. This hook only keeps the
 * tab strip and omnibox in sync and forwards explicit actions.
 */
export function useWebviewBrowser(workspaceId: string | null) {
  const [page, setPage] = useState<BrowserPageState>(EMPTY_PAGE);
  const [tabs, setTabs] = useState<BrowserTabView[]>([]);
  const [navigating, setNavigating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const workspaceRef = useRef(workspaceId);
  workspaceRef.current = workspaceId;
  const tabsRef = useRef<BrowserTabView[]>([]);
  tabsRef.current = tabs;
  const restoredFor = useRef<string | null>(null);

  const refreshInfo = useCallback(async () => {
    if (!workspaceId) return null;
    const requested = workspaceId;
    try {
      const out = await api.browserInvoke({
        workspace_id: requested,
        action: "info",
      });
      if (workspaceRef.current !== requested) return null;
      if (!out.ok) {
        if (out.error) setError(out.error);
        return null;
      }
      const payload = out.payload as BrowserInvokePayload | null;
      const pages = Array.isArray(payload?.pages) ? payload.pages : [];
      setTabs(pages);
      const meaningful = pages.filter(
        (tab) => tab.url && tab.url !== "about:blank",
      );
      if (meaningful.length > 0) {
        saveBrowserSession(workspaceId, {
          tabs: meaningful.map((tab) => ({
            url: tab.url ?? "",
            title: tab.title ?? "",
          })),
          activeIndex: Math.max(
            0,
            meaningful.findIndex((tab) => tab.active),
          ),
          savedAt: Date.now(),
        });
      }
      setPage((current) => ({
        url: payload?.active?.url ?? current.url,
        title: payload?.active?.title ?? current.title,
        canGoBack: payload?.can_go_back ?? false,
        canGoForward: payload?.can_go_forward ?? false,
      }));
      setError(null);
      return pages;
    } catch (cause) {
      if (workspaceRef.current === requested) setError(errorMessage(cause));
      return null;
    }
  }, [workspaceId]);

  useEffect(() => {
    setPage(EMPTY_PAGE);
    setTabs([]);
    setError(null);
    if (!workspaceId) return;
    void refreshInfo();
    const id = window.setInterval(() => void refreshInfo(), 1_000);
    let unlisten: (() => void) | undefined;
    void api
      .browserStateEvent((event) => {
        if (event.workspace_id === workspaceRef.current) void refreshInfo();
      })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      window.clearInterval(id);
      unlisten?.();
    };
  }, [refreshInfo, workspaceId]);

  // Recreate the saved tab set only when the backend has no meaningful page.
  useEffect(() => {
    if (!workspaceId || restoredFor.current === workspaceId) return;
    restoredFor.current = workspaceId;
    const snapshot = readBrowserSession(workspaceId);
    if (!snapshot || snapshot.tabs.length === 0) return;
    const restore = async () => {
      const current = await refreshInfo();
      if (
        !current ||
        current.some((tab) => tab.url && tab.url !== "about:blank") ||
        current.length === 0
      ) {
        return;
      }
      await run("navigate", { url: snapshot.tabs[0]?.url });
      for (const tab of snapshot.tabs.slice(1)) {
        await run("new_tab", { url: tab.url });
      }
      const target = snapshot.tabs[snapshot.activeIndex];
      if (target) {
        const pages = await refreshInfo();
        const match = pages?.find((tab) => tab.url === target.url);
        if (match) await run("switch_tab", { tab_id: match.id });
      }
    };
    void restore();
    // `run` is intentionally omitted: this effect is a one-shot per workspace.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshInfo, workspaceId]);

  const run = useCallback(
    async (action: string, extra: Record<string, unknown> = {}) => {
      if (!workspaceId) return;
      const requested = workspaceId;
      setNavigating(true);
      try {
        const out = await api.browserInvoke({
          workspace_id: requested,
          action,
          ...extra,
        });
        if (workspaceRef.current !== requested) return;
        if (!out.ok && out.error) setError(out.error);
        await refreshInfo();
      } catch (cause) {
        if (workspaceRef.current === requested) setError(errorMessage(cause));
      } finally {
        if (workspaceRef.current === requested) setNavigating(false);
      }
    },
    [refreshInfo, workspaceId],
  );

  const setSurfaceBounds = useCallback(
    (bounds: {
      x: number;
      y: number;
      width: number;
      height: number;
      visible: boolean;
    }) => {
      if (!workspaceId) return;
      void api.browserSurfaceBounds(workspaceId, bounds).catch(() => undefined);
    },
    [workspaceId],
  );

  const clearData = useCallback(async () => {
    if (!workspaceId) return;
    await api.browserClearData(workspaceId);
    clearBrowserSession(workspaceId);
    setPage(EMPTY_PAGE);
    setTabs([]);
  }, [workspaceId]);

  const closeTab = useCallback(
    async (id: string) => {
      if (tabs.length <= 1) {
        if (workspaceId) clearBrowserSession(workspaceId);
        await run("navigate", { url: "about:blank" });
        return;
      }
      await run("close_tab", { tab_id: id });
    },
    [run, tabs.length, workspaceId],
  );

  return {
    page,
    tabs,
    navigating,
    error,
    refresh: refreshInfo,
    clearError: () => setError(null),
    setSurfaceBounds,
    clearData,
    navigate: (url: string) => run("navigate", { url }),
    reload: () => run("reload"),
    back: () => run("back"),
    forward: () => run("forward"),
    stop: async () => {
      if (!workspaceId) return;
      setNavigating(false);
      await api.browserStop(workspaceId).catch(() => undefined);
      await refreshInfo();
    },
    newTab: (url?: string) => run("new_tab", { url: url ?? null }),
    switchTab: (id: string) => run("switch_tab", { tab_id: id }),
    closeTab,
    closeSession: async () => {
      if (workspaceId) clearBrowserSession(workspaceId);
      await run("close");
    },
  };
}

export type WebviewBrowser = ReturnType<typeof useWebviewBrowser>;
