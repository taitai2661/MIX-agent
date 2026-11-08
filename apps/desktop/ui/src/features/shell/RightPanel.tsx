import { useEffect, useRef, useState, type Ref } from "react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { recordUxEvent } from "@/lib/uxEvents";
import {
  DEFAULT_BROWSER_PANEL_RATIO,
  MAX_RIGHT_PANEL_WIDTH,
  MIN_RIGHT_PANEL_WIDTH,
  useUi,
} from "@/lib/ui";
import { canFullView, isRightPanelTab } from "@/lib/panelTabs";
import { RightPanelBody } from "./RightPanelBody";
import { RightPanelHeader } from "./RightPanelHeader";
import { panelStyle } from "./usePanelCap";
import { ResizableHandle } from "./ResizableHandle";

/** Below this width the tab bar shows only the selected tab's label. */
const COMPACT_TABS_BELOW = 460;

export function RightPanel({
  rightMax = null,
  rowWidth = 0,
}: {
  rightMax?: number | null;
  rowWidth?: number;
}) {
  const rightPanel = useUi((s) => s.rightPanel);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const surface = useUi((s) => s.surface);
  const isAgentSurface = surface === "desktop-agent";
  const rightPanelWidth = useUi((s) => s.rightPanelWidth);
  const rightPanelRatio = useUi((s) => s.rightPanelRatio);
  const setRightPanelRatio = useUi((s) => s.setRightPanelRatio);
  const isDocked = useUi((s) => s.rightPanelDocked);
  const sidePanelMode = useUi((s) => s.sidePanelMode);
  const diffTarget = useUi((s) => s.diffTarget);
  const filePreviewPath = useUi((s) => s.filePreviewPath);
  const setRightPanelWidth = useUi((s) => s.setRightPanelWidth);
  const asideRef = useRef<HTMLElement | null>(null);
  // Keep drag updates local: the store writes to localStorage on every setter
  // call, and pointermove can fire far more often than the persisted value
  // needs to change. Commit once when the drag ends.
  const [resizingWidth, setResizingWidth] = useState<number | null>(null);
  // Full view keeps this same `<aside>` mounted and lets it take the centre
  // column. Rendering the body in a second React tree (as the first version
  // did) unmounted the browser on every split/full toggle.
  const isFull =
    sidePanelMode === "full" && canFullView(rightPanel, isAgentSurface);
  // The browser pane stores a share of the shell width, so a window resize
  // keeps the split instead of freezing at yesterday's pixel count. Other
  // panels keep their pixel width (their content is not proportional).
  const ratioWidth =
    rightPanel === "browser" && rightPanelRatio !== null && rowWidth > 0
      ? Math.round(rowWidth * rightPanelRatio)
      : null;
  const storedWidth = ratioWidth ?? rightPanelWidth;
  const panelMax = isDocked ? (rightMax ?? MAX_RIGHT_PANEL_WIDTH) : null;
  const panelWidthValue =
    resizingWidth ??
    (panelMax === null ? storedWidth : Math.min(storedWidth, panelMax));

  // `rightPanel` is persisted but its targets are session-only. After a
  // restart with "diff"/"file" stored, the panel restored as a 380px blank
  // column with no header and no close button — indistinguishable from a
  // rendering bug. Drop the panel when its target no longer exists.
  useEffect(() => {
    if (rightPanel === "diff" && !diffTarget) setRightPanel("none");
    if (rightPanel === "file" && !filePreviewPath) setRightPanel("none");
  }, [diffTarget, filePreviewPath, rightPanel, setRightPanel]);

  // The chat surface has no file tree or workspace targets. A panel left over
  // from the agent surface (or restored from localStorage) would render an
  // empty workspace column, so land on the usage panel — the one panel that
  // matters in a plain conversation — instead.
  useEffect(() => {
    if (isAgentSurface) return;
    if (
      rightPanel === "changes" ||
      rightPanel === "browser" ||
      rightPanel === "diff" ||
      rightPanel === "file"
    ) {
      setRightPanel("usage");
    }
  }, [isAgentSurface, rightPanel, setRightPanel]);

  // Overlay drawer: a click outside closes it. It is deliberately non-modal:
  // opening it must not steal focus from the composer, and Tab is free to leave
  // the panel. Escape is handled once in `useAppShortcuts`, which knows about
  // the palette, shortcut sheet and native popups above the panel; a second
  // bubble-phase handler here closed the drawer while one of those was closing.
  useEffect(() => {
    if (rightPanel === "none" || isDocked || isFull) return;
    const onPointerDown = (e: PointerEvent) => {
      const aside = asideRef.current;
      if (aside && !aside.contains(e.target as Node)) {
        useUi.getState().closeRightPanel();
      }
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [isDocked, isFull, rightPanel]);

  useEffect(() => {
    if (rightPanel === "none") return;
    recordUxEvent("panel_opened", { panel: rightPanel, surface });
  }, [rightPanel, surface]);

  if (rightPanel === "none") return null;

  const tabbed = isRightPanelTab(rightPanel);

  const commitWidth = (next: number) => {
    setResizingWidth(null);
    setRightPanelWidth(next);
    // A user drag takes over from an agent-specified split. Remember the
    // share, not just the pixel width, so the page stays the same relative
    // size when the window changes.
    if (rightPanel === "browser" && rowWidth > 0) {
      setRightPanelRatio(next / rowWidth);
    }
  };
  const resetBrowserRatio = () => {
    setResizingWidth(null);
    setRightPanelRatio(DEFAULT_BROWSER_PANEL_RATIO);
  };
  const resizeHandle = isFull ? null : (
    <ResizableHandle
      value={panelWidthValue}
      min={MIN_RIGHT_PANEL_WIDTH}
      max={panelMax ?? MAX_RIGHT_PANEL_WIDTH}
      direction={-1}
      label={t("サイドパネルの幅を変更")}
      onChange={setResizingWidth}
      onCommit={commitWidth}
      onReset={rightPanel === "browser" ? resetBrowserRatio : undefined}
      edge="left"
    />
  );

  // Can be rendered either as a docked flex sibling (split-view), as an
  // overlay drawer, or expanded into the centre column (full view). The
  // expanded state is still the same element in the same tree, so switching
  // between split and full never remounts the native browser WebView.
  const effectiveDocked = isFull || isDocked;
  return (
    <aside
      ref={asideRef as Ref<HTMLElement>}
      tabIndex={-1}
      data-side-panel="right"
      data-side-panel-mode={isFull ? "full" : "split"}
      style={
        isFull
          ? undefined
          : panelStyle(panelWidthValue, isDocked ? panelMax : null)
      }
      role={effectiveDocked ? "region" : "dialog"}
      aria-modal={effectiveDocked ? undefined : false}
      aria-label={t("サイドパネル")}
      className={cn(
        isFull
          ? "relative z-10 flex min-w-0 flex-1 flex-col overflow-hidden border-l border-line bg-bg animate-fade"
          : isDocked
            ? "relative z-10 flex shrink-0 flex-col overflow-hidden border-l border-line bg-bg animate-fade"
            : "absolute inset-y-0 right-0 z-30 flex max-w-[85vw] flex-col overflow-hidden border-l border-line bg-bg shadow-pop animate-fade",
        // Preset changes (wide/normal) morph instead of jumping. Dragging must
        // stay 1:1 with the pointer, so the transition is disabled mid-drag.
        !isFull &&
          resizingWidth === null &&
          "transition-[width,max-width] duration-200 ease-out motion-reduce:transition-none",
      )}
    >
      {resizeHandle}
      {/* One header owns the tabs and the window controls. The browser no
          longer pushes a second app-level bar above its page tabs. */}
      {tabbed && (
        <RightPanelHeader
          compact={!isFull && panelWidthValue < COMPACT_TABS_BELOW}
        />
      )}
      <div
        key={rightPanel}
        className="flex min-h-0 flex-1 flex-col animate-fade"
      >
        <RightPanelBody panel={rightPanel} isAgentSurface={isAgentSurface} />
      </div>
    </aside>
  );
}
