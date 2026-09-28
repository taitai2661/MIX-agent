import { useCallback, useEffect, useRef, useState } from "react";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { ActivityPanel } from "@/features/chat/ActivityPanel";
import { DiffPanel } from "@/features/chat/DiffView";
import { ChangesPanel, FilePreviewPanel } from "@/features/files/ChangesPanel";

const WIDTH_KEY = "mix-agent-right-panel-width";
const MIN_WIDTH = 280;
const MAX_WIDTH = 720;

export function RightPanel() {
  const rightPanel = useUi((s) => s.rightPanel);
  const [width, setWidth] = useState(() => {
    const stored = Number(localStorage.getItem(WIDTH_KEY));
    return Number.isFinite(stored) && stored >= MIN_WIDTH && stored <= MAX_WIDTH
      ? stored
      : 360;
  });
  const [dragging, setDragging] = useState(false);
  const draggingRef = useRef(false);

  const onMove = useCallback((e: MouseEvent) => {
    if (!draggingRef.current) return;
    const next = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, window.innerWidth - e.clientX));
    setWidth(next);
  }, []);

  const onUp = useCallback(() => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    setDragging(false);
    document.body.style.cursor = "";
    try {
      localStorage.setItem(WIDTH_KEY, String(width));
    } catch {
      // best-effort
    }
  }, [width]);

  useEffect(() => {
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [onMove, onUp]);

  if (rightPanel === "none") return null;

  return (
    <>
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="リサイズ"
        data-dragging={dragging}
        onMouseDown={() => {
          draggingRef.current = true;
          setDragging(true);
          document.body.style.cursor = "col-resize";
        }}
        className="w-1 shrink-0 cursor-col-resize bg-transparent transition-colors duration-150 hover:bg-brand-soft data-[dragging=true]:bg-brand-soft"
      />
      <aside
        style={{ width }}
        aria-label={t("このRunのアクティビティ")}
        className="flex shrink-0 flex-col overflow-hidden border-l border-line bg-bg"
      >
        {rightPanel === "diff" && <DiffPanel />}
        {rightPanel === "changes" && <ChangesPanel />}
        {rightPanel === "file" && <FilePreviewPanel />}
        {(rightPanel === "activity") && <ActivityPanel />}
      </aside>
    </>
  );
}
