import { useCallback, useEffect, useState } from "react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import type { BrowserFrameView } from "@/lib/types";

const POLL_MS = 1500;

/**
 * Live view: poll a non-persisted JPEG frame and send manual input.
 *
 * The Rust side returns `null` when the workspace is busy with an agent action;
 * skipping that tick keeps the screenshot from delaying the action. Frames
 * also stop while the window is hidden — leaving a 1.5 s capture loop running
 * in the background burns CPU for an image nobody can see.
 */
export function useLiveFrame(workspaceId: string) {
  const [live, setLive] = useState(false);
  const [frameSrc, setFrameSrc] = useState<string | null>(null);
  const [frameSize, setFrameSize] = useState({ width: 0, height: 0 });
  const [frameError, setFrameError] = useState<string | null>(null);
  const [liveText, setLiveText] = useState("");

  const applyFrame = useCallback((frame: BrowserFrameView | null) => {
    if (!frame) return;
    setFrameSrc(frame.data);
    setFrameSize({ width: frame.width, height: frame.height });
    setFrameError(null);
  }, []);

  const refreshFrame = useCallback(async () => {
    try {
      applyFrame(await api.browserFrame(workspaceId, false));
    } catch (e) {
      setFrameError(errorMessage(e));
    }
  }, [applyFrame, workspaceId]);

  useEffect(() => {
    if (!live) return;
    let cancelled = false;
    let inFlight = false;
    const tick = async () => {
      if (inFlight || document.hidden) return;
      inFlight = true;
      try {
        const frame = await api.browserFrame(workspaceId, false);
        if (!cancelled) applyFrame(frame);
      } catch (e) {
        if (!cancelled) setFrameError(errorMessage(e));
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
      cancelled = true;
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [live, workspaceId, applyFrame]);

  // A different workspace must not briefly show the previous one's cookies.
  useEffect(() => {
    setFrameSrc(null);
    setFrameError(null);
  }, [workspaceId]);

  const runManual = useCallback(
    async (input: {
      action: "click_at" | "type_active" | "press";
      x?: number;
      y?: number;
      text?: string;
      key?: string;
    }) => {
      try {
        await api.browserInvoke({ workspace_id: workspaceId, ...input });
        await refreshFrame();
      } catch (e) {
        setFrameError(errorMessage(e));
      }
    },
    [refreshFrame, workspaceId],
  );

  const clearFrame = useCallback(() => {
    setFrameSrc(null);
    setFrameError(null);
  }, []);

  return {
    live,
    setLive,
    frameSrc,
    frameSize,
    frameError,
    liveText,
    setLiveText,
    refreshFrame,
    clearFrame,
    clickAt: (x: number, y: number) => runManual({ action: "click_at", x, y }),
    typeActive: (text: string) => runManual({ action: "type_active", text }),
    pressKey: (key: string) => runManual({ action: "press", key }),
  };
}
