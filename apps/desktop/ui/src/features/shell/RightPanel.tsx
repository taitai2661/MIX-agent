import { Suspense, lazy, useEffect, useRef, useState, type Ref } from "react";
import {
  Activity,
  Columns2,
  Gauge,
  GitBranch,
  Globe,
  Maximize2,
  Minimize2,
  PanelRight,
  X,
} from "lucide-react";
import { t } from "@/lib/i18n";
import {
  MAX_RIGHT_PANEL_WIDTH,
  MIN_RIGHT_PANEL_WIDTH,
  useUi,
  WIDE_RIGHT_PANEL_WIDTH,
} from "@/lib/ui";
import { ActivityPanel } from "@/features/chat/ActivityPanel";
import { UsagePanel } from "@/features/chat/UsagePanel";
import { DiffPanel } from "@/features/chat/DiffView";
import { ChangesPanel, FilePreviewPanel } from "@/features/files/ChangesPanel";
import { Spinner } from "@/components/ui/spinner";
import { Button } from "@/components/ui/button";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";
import { panelStyle } from "./usePanelCap";
import { ResizableHandle } from "./ResizableHandle";

const BrowserPanel = lazy(() =>
  import("@/features/browser/BrowserPanel").then((m) => ({
    default: m.BrowserPanel,
  })),
);

export function RightPanel({ cap = null }: { cap?: number | null }) {
  const rightPanel = useUi((s) => s.rightPanel);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const surface = useUi((s) => s.surface);
  const isAgentSurface = surface === "desktop-agent";
  const dismissBrowserPanel = useUi((s) => s.dismissBrowserPanel);
  const openBrowserPanel = useUi((s) => s.openBrowserPanel);
  const rightPanelWidth = useUi((s) => s.rightPanelWidth);
  const toggleRightPanelWidth = useUi((s) => s.toggleRightPanelWidth);
  const isDocked = useUi((s) => s.rightPanelDocked);
  const toggleDocked = useUi((s) => s.toggleRightPanelDocked);
  const isWide = rightPanelWidth >= WIDE_RIGHT_PANEL_WIDTH;
  const diffTarget = useUi((s) => s.diffTarget);
  const filePreviewPath = useUi((s) => s.filePreviewPath);
  const setRightPanelWidth = useUi((s) => s.setRightPanelWidth);
  const asideRef = useRef<HTMLElement | null>(null);
  // Keep drag updates local: the store writes to localStorage on every setter
  // call, and pointermove can fire far more often than the persisted value
  // needs to change. Commit once when the drag ends.
  const [resizingWidth, setResizingWidth] = useState<number | null>(null);
  const panelWidthValue = resizingWidth ?? rightPanelWidth;

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
    if (rightPanel === "none" || isDocked) return;
    const onPointerDown = (e: PointerEvent) => {
      const aside = asideRef.current;
      if (aside && !aside.contains(e.target as Node)) setRightPanel("none");
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [isDocked, rightPanel, setRightPanel]);

  if (rightPanel === "none") return null;

  const tabbed =
    rightPanel === "activity" ||
    rightPanel === "usage" ||
    rightPanel === "changes" ||
    rightPanel === "browser";

  const commitWidth = (next: number) => {
    setResizingWidth(null);
    setRightPanelWidth(next);
  };
  const resizeHandle = (
    <ResizableHandle
      value={panelWidthValue}
      min={MIN_RIGHT_PANEL_WIDTH}
      max={MAX_RIGHT_PANEL_WIDTH}
      direction={-1}
      label={t("サイドパネルの幅を変更")}
      onChange={setResizingWidth}
      onCommit={commitWidth}
      edge="left"
    />
  );

  // Can be rendered either as a docked flex sibling (split-view) or as an
  // overlay drawer depending on `isDocked`. An overlay drawer is out of flow:
  // it takes no width from the centre column, so the inline-panel cap must not
  // squeeze it (or its siblings).
  return (
    <aside
      ref={asideRef as Ref<HTMLElement>}
      tabIndex={-1}
      data-side-panel=""
      style={panelStyle(panelWidthValue, isDocked ? cap : null)}
      role={isDocked ? "region" : "dialog"}
      aria-modal="false"
      aria-label={t("サイドパネル")}
      className={
        isDocked
          ? "relative z-10 flex shrink-0 flex-col overflow-hidden border-l border-line bg-bg animate-fade"
          : "absolute inset-y-0 right-0 z-30 flex max-w-[85vw] flex-col overflow-hidden border-l border-line bg-bg shadow-pop animate-fade"
      }
    >
      {resizeHandle}
      {tabbed && (
        <div className="flex h-titlebar shrink-0 items-center gap-1.5 border-b border-line bg-bg/85 px-3 backdrop-blur-md">
          {/* Named differently from the drawer's own `サイドパネル` label:
              two nested controls with the identical accessible name read as
              duplicates to screen readers. */}
          <Segmented
            aria-label={t("パネル切り替え")}
            className="min-w-0 flex-1"
          >
            <SegmentedOption
              value="activity"
              label={t("このRunのアクティビティ")}
              icon={<Activity size={13} aria-hidden="true" />}
              selected={rightPanel === "activity"}
              onSelect={() => setRightPanel("activity")}
            >
              {t("アクティビティ")}
            </SegmentedOption>
            <SegmentedOption
              value="usage"
              label={t("使用状況")}
              icon={<Gauge size={13} aria-hidden="true" />}
              selected={rightPanel === "usage"}
              onSelect={() => setRightPanel("usage")}
            >
              {t("使用")}
            </SegmentedOption>
            {isAgentSurface && (
              <SegmentedOption
                value="changes"
                label={t("変更")}
                icon={<GitBranch size={13} aria-hidden="true" />}
                selected={rightPanel === "changes"}
                onSelect={() => setRightPanel("changes")}
              >
                {t("変更")}
              </SegmentedOption>
            )}
            {isAgentSurface && (
              <SegmentedOption
                value="browser"
                label={t("ブラウザ")}
                icon={<Globe size={13} aria-hidden="true" />}
                selected={rightPanel === "browser"}
                onSelect={() => openBrowserPanel("user")}
              >
                {t("ブラウザ")}
              </SegmentedOption>
            )}
          </Segmented>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={toggleDocked}
            title={
              isDocked ? t("オーバーレイ表示に切り替え") : t("右側に固定表示")
            }
            aria-label={
              isDocked ? t("オーバーレイ表示に切り替え") : t("右側に固定表示")
            }
            aria-pressed={isDocked}
          >
            {isDocked ? (
              <PanelRight size={13} className="text-brand" />
            ) : (
              <Columns2 size={13} />
            )}
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={toggleRightPanelWidth}
            title={isWide ? t("標準幅に戻す") : t("パネル幅を拡大")}
            aria-label={isWide ? t("標準幅に戻す") : t("パネル幅を拡大")}
          >
            {isWide ? <Minimize2 size={13} /> : <Maximize2 size={13} />}
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() =>
              rightPanel === "browser"
                ? dismissBrowserPanel()
                : setRightPanel("none")
            }
            title={t("閉じる")}
            aria-label={t("閉じる")}
          >
            <X size={14} />
          </Button>
        </div>
      )}
      {rightPanel === "diff" && isAgentSurface && <DiffPanel />}
      {rightPanel === "changes" && isAgentSurface && <ChangesPanel embedded />}
      {rightPanel === "file" && isAgentSurface && <FilePreviewPanel />}
      {rightPanel === "activity" && <ActivityPanel embedded />}
      {rightPanel === "usage" && <UsagePanel embedded />}
      {rightPanel === "browser" && isAgentSurface && (
        <div className="flex min-h-0 flex-1 flex-col">
          <Suspense
            fallback={
              <div className="flex flex-1 items-center justify-center">
                <Spinner size={14} label={t("読み込み中…")} />
              </div>
            }
          >
            <BrowserPanel />
          </Suspense>
        </div>
      )}
    </aside>
  );
}
