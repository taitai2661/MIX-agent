import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import type {
  BrowserFrameView,
  BrowserScreencastFrame,
  BrowserTabView,
} from "@/lib/types";

const POLL_MS = 2_000;
/** If the screencast has been silent this long, fall back to snapshots. */
const FALLBACK_AFTER_MS = 3_000;

/**
 * The CDP screencast belongs to the workspace, not to one hook instance: the
 * Settings pane and the thread side panel can both show a live view for the
 * same workspace. Ref-count the consumers so closing one pane does not stop
 * the stream the other one is still reading.
 */
const screencastConsumers = new Map<string, number>();

async function acquireScreencast(workspaceId: string) {
  const users = screencastConsumers.get(workspaceId) ?? 0;
  screencastConsumers.set(workspaceId, users + 1);
  if (users === 0) await api.browserScreencastStart(workspaceId);
}

async function releaseScreencast(workspaceId: string) {
  const users = screencastConsumers.get(workspaceId) ?? 0;
  if (users > 1) {
    screencastConsumers.set(workspaceId, users - 1);
    return;
  }
  screencastConsumers.delete(workspaceId);
  await api.browserScreencastStop(workspaceId);
}

/** Restart after the active tab changed without disturbing the ref count. */
async function restartScreencast(workspaceId: string) {
  if (!screencastConsumers.has(workspaceId)) return;
  await api.browserScreencastStop(workspaceId);
  await api.browserScreencastStart(workspaceId);
}

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

/**
 * Live view: CDP screencast when available, 2 s snapshot polling as fallback.
 *
 * The Rust side acknowledges screencast frames and emits `browser://frame`.
 * If no frame arrives (older backend, screencast disabled, a page that cannot
 * be captured) the same poll keeps the panel updated. Frames stop while the
 * window is hidden and no capture runs while the workspace is busy with an
 * agent action (`browser_frame` returns null), so the panel cannot slow the
 * agent down.
 */
export function useLiveFrame(
  workspaceId: string | null,
  options: { autoStart?: boolean } = {},
) {
  const [live, setLive] = useState(false);
  const [frameSrc, setFrameSrc] = useState<string | null>(null);
  const [frameSize, setFrameSize] = useState({ width: 0, height: 0 });
  const [frameError, setFrameError] = useState<string | null>(null);
  const [liveText, setLiveText] = useState("");
  const [page, setPage] = useState<BrowserPageState>(EMPTY_PAGE);
  const [tabs, setTabs] = useState<BrowserTabView[]>([]);

  const lastScreencast = useRef(0);
  const workspaceRef = useRef(workspaceId);
  workspaceRef.current = workspaceId;

  const applyFrame = useCallback((frame: BrowserFrameView | null) => {
    if (!frame) return;
    setFrameSrc(frame.data);
    setFrameSize({ width: frame.width, height: frame.height });
    setFrameError(null);
  }, []);

  const refreshFrame = useCallback(async () => {
    if (!workspaceId) return;
    const requested = workspaceId;
    try {
      const frame = await api.browserFrame(requested, false);
      // The user may switch workspaces while the capture is in flight. The
      // reset effect cannot cancel a promise, so drop a late result here.
      if (workspaceRef.current !== requested) return;
      applyFrame(frame);
    } catch (e) {
      if (workspaceRef.current !== requested) return;
      setFrameError(errorMessage(e));
    }
  }, [applyFrame, workspaceId]);

  /** URL/title/history and tab strip. `info` is much cheaper than a capture. */
  const refreshInfo = useCallback(async () => {
    if (!workspaceId) return;
    const requested = workspaceId;
    try {
      const out = await api.browserInvoke({
        workspace_id: requested,
        action: "info",
      });
      if (workspaceRef.current !== requested) return;
      if (!out.ok) {
        if (out.error) setFrameError(out.error);
        return;
      }
      const payload = out.payload as {
        active?: {
          id?: string;
          url?: string | null;
          title?: string | null;
        };
        pages?: BrowserTabView[];
        can_go_back?: boolean;
        can_go_forward?: boolean;
      } | null;
      if (!payload) return;
      setPage((current) => ({
        url: payload.active?.url ?? current.url,
        title: payload.active?.title ?? current.title,
        canGoBack: payload.can_go_back ?? current.canGoBack,
        canGoForward: payload.can_go_forward ?? current.canGoForward,
      }));
      if (Array.isArray(payload.pages)) setTabs(payload.pages);
      setFrameError(null);
    } catch (e) {
      if (workspaceRef.current !== requested) return;
      setFrameError(errorMessage(e));
    }
  }, [workspaceId]);

  // Subscribe to screencast frames only while live.
  useEffect(() => {
    if (!live || !workspaceId) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void api
      .browserFrameEvent((frame: BrowserScreencastFrame) => {
        if (disposed || frame.workspace_id !== workspaceRef.current) return;
        lastScreencast.current = Date.now();
        setFrameSrc(frame.data);
        setFrameSize({ width: frame.width, height: frame.height });
        setFrameError(null);
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // A shell without the event still gets the polling fallback below.
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [live, workspaceId]);

  // Start/stop the screencast with the live toggle. The workspace owns one
  // stream; consumers are ref-counted so two panes can share it.
  useEffect(() => {
    if (!live || !workspaceId) return;
    // Frames may start arriving immediately; without a fresh timestamp the
    // fallback poll would also take a full snapshot on its first tick.
    lastScreencast.current = Date.now();
    void acquireScreencast(workspaceId).catch(() => {
      // Polling is the fallback; a rejected screencast must not leave the
      // panel stuck on a spinner.
    });
    // Draw a frame immediately instead of waiting for the first event.
    void refreshFrame();
    return () => {
      void releaseScreencast(workspaceId).catch(() => undefined);
    };
  }, [live, refreshFrame, workspaceId]);

  // Metadata + capture fallback loop.
  useEffect(() => {
    if (!live || !workspaceId) return;
    let inFlight = false;
    const tick = async () => {
      if (inFlight || document.hidden) return;
      inFlight = true;
      try {
        await refreshInfo();
        if (Date.now() - lastScreencast.current > FALLBACK_AFTER_MS) {
          await refreshFrame();
        }
      } finally {
        inFlight = false;
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), POLL_MS);
    const onVisibility = () => {
      if (!document.hidden) void tick();
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [live, refreshFrame, refreshInfo, workspaceId]);

  // An open pane keeps its workspace out of the idle eviction even while the
  // live view is paused. `browser_keepalive` only updates the Rust LRU
  // timestamp, so it never launches Chromium just to stay alive.
  useEffect(() => {
    if (!workspaceId) return;
    let cancelled = false;
    const ping = () => {
      if (!cancelled)
        void api.browserKeepalive(workspaceId).catch(() => undefined);
    };
    ping();
    const id = window.setInterval(ping, 60_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [workspaceId]);

  // Auto-start for a panel that was opened because the agent is browsing.
  useEffect(() => {
    if (options.autoStart && workspaceId) setLive(true);
  }, [options.autoStart, workspaceId]);

  // A different workspace must not briefly show the previous one's cookies.
  useEffect(() => {
    setFrameSrc(null);
    setFrameError(null);
    setPage(EMPTY_PAGE);
    setTabs([]);
  }, [workspaceId]);

  const runManual = useCallback(
    async (input: {
      action: "click_at" | "type_active" | "press";
      x?: number;
      y?: number;
      text?: string;
      key?: string;
    }) => {
      if (!workspaceId) return;
      const requested = workspaceId;
      try {
        const out = await api.browserInvoke({
          workspace_id: requested,
          ...input,
        });
        if (workspaceRef.current !== requested) return;
        if (!out.ok && out.error) setFrameError(out.error);
        await Promise.all([refreshFrame(), refreshInfo()]);
      } catch (e) {
        if (workspaceRef.current !== requested) return;
        setFrameError(errorMessage(e));
      }
    },
    [refreshFrame, refreshInfo, workspaceId],
  );

  const runAction = useCallback(
    async (action: string, extra: Record<string, unknown> = {}) => {
      if (!workspaceId) return;
      const requested = workspaceId;
      // Navigating from the address bar or a tab click must show the result,
      // even when the user opened the pane without starting the live view.
      setLive(true);
      try {
        const out = await api.browserInvoke({
          workspace_id: requested,
          action,
          ...extra,
        });
        if (workspaceRef.current !== requested) return;
        if (!out.ok && out.error) setFrameError(out.error);
        // The backend restarts the screencast for agent-driven tab changes;
        // the toolbar's own actions restart it here so the very next frame
        // comes from the new target. Restarting keeps the consumer count, so
        // another pane reading the same workspace keeps its stream too.
        if (
          action === "switch_tab" ||
          action === "new_tab" ||
          action === "close_tab"
        ) {
          await restartScreencast(requested);
        }
        await Promise.all([refreshFrame(), refreshInfo()]);
      } catch (e) {
        if (workspaceRef.current !== requested) return;
        setFrameError(errorMessage(e));
      }
    },
    [refreshFrame, refreshInfo, workspaceId],
  );

  const clearFrame = useCallback(() => {
    setFrameSrc(null);
    setFrameError(null);
  }, []);

  /** Refresh both the picture and the page metadata. */
  const refresh = useCallback(
    () => Promise.all([refreshFrame(), refreshInfo()]),
    [refreshFrame, refreshInfo],
  );

  return {
    live,
    setLive,
    frameSrc,
    frameSize,
    frameError,
    liveText,
    setLiveText,
    page,
    tabs,
    refreshFrame,
    refreshInfo,
    refresh,
    clearFrame,
    clickAt: (x: number, y: number) => runManual({ action: "click_at", x, y }),
    typeActive: (text: string) => runManual({ action: "type_active", text }),
    pressKey: (key: string) => runManual({ action: "press", key }),
    navigate: (url: string) => runAction("navigate", { url }),
    reload: () => runAction("reload"),
    back: () => runAction("back"),
    forward: () => runAction("forward"),
    stop: async () => {
      if (!workspaceId) return;
      try {
        await api.browserStop(workspaceId);
        await refreshInfo();
      } catch (e) {
        setFrameError(errorMessage(e));
      }
    },
    newTab: (url?: string) => runAction("new_tab", { url: url ?? null }),
    switchTab: (tabId: string) => runAction("switch_tab", { tab_id: tabId }),
    closeTab: (tabId: string) => runAction("close_tab", { tab_id: tabId }),
  };
}
