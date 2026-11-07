import { useEffect, useState } from "react";
import { Globe, Maximize2, Minimize2, X } from "lucide-react";
import { t } from "@/lib/i18n";
import { DEFAULT_BROWSER_PANEL_RATIO, useUi } from "@/lib/ui";
import { useRunEvents } from "@/lib/runStore";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import { BrowserSurface } from "./BrowserSurface";
import { BrowserTabStrip } from "./BrowserTabStrip";
import { BrowserToolbar } from "./BrowserToolbar";
import { useWebviewBrowser } from "./useWebviewBrowser";

/** Minimal chrome for the states that cannot show the page toolbar (missing
 *  workspace). Full view has no `RightPanel` header, so the close/full-view
 *  controls still need somewhere to live. */
function PanelChrome({
  full,
  onToggleFull,
  onClose,
  children,
}: {
  full: boolean;
  onToggleFull: () => void;
  onClose: () => void;
  children: React.ReactNode;
}) {
  return (
    <div data-browser-pane className="flex h-full min-h-0 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-1 border-b border-line bg-bg/85 px-2">
        <Globe size={13} className="text-brand" />
        <span className="flex-1 text-xs font-medium text-fg">
          {t("ブラウザ")}
        </span>
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={onToggleFull}
          title={full ? t("スプリット表示に戻す") : t("フル表示")}
          aria-label={full ? t("スプリット表示に戻す") : t("フル表示")}
        >
          {full ? <Minimize2 size={13} /> : <Maximize2 size={13} />}
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={onClose}
          title={t("閉じる")}
          aria-label={t("閉じる")}
        >
          <X size={14} />
        </Button>
      </div>
      {children}
    </div>
  );
}

/**
 * The thread side-panel browser. The page is a native Tauri child WebView;
 * React owns only the tab strip, the omnibox and the surface rectangle.
 */
export function BrowserPanel() {
  const workspaceId = useUi((s) => s.workspaceId);
  const openedBy = useUi((s) => s.browserOpenedBy);
  const autoOpen = useUi((s) => s.browserAutoOpen);
  const setBrowserAutoOpen = useUi((s) => s.setBrowserAutoOpen);
  const full = useUi((s) => s.sidePanelMode === "full");
  const docked = useUi((s) => s.rightPanelDocked);
  const toggleSidePanelMode = useUi((s) => s.toggleSidePanelMode);
  const toggleDocked = useUi((s) => s.toggleRightPanelDocked);
  const dismissBrowserPanel = useUi((s) => s.dismissBrowserPanel);
  const openSettings = useUi((s) => s.openSettings);
  const conversationId = useUi((s) => s.conversationId);
  const setRightPanelRatio = useUi((s) => s.setRightPanelRatio);
  const [overlayOpen, setOverlayOpen] = useState(false);
  const { confirm, dialog } = useConfirm();

  const browserToolRunning = useRunEvents((s) => {
    if (!conversationId) return false;
    return (
      s.byConversation[conversationId]?.timeline.tools.some(
        (tool) => tool.name === "browser" && tool.status === "running",
      ) ?? false
    );
  });

  const browser = useWebviewBrowser(workspaceId);
  const setSurfaceBounds = browser.setSurfaceBounds;

  // The native WebView owns most of the screen; hide it while a React menu or
  // confirmation dialog needs to paint above the page.
  useEffect(() => {
    if (!overlayOpen) return;
    const node = document.querySelector("[data-browser-surface]");
    if (!node) return;
    const rect = node.getBoundingClientRect();
    setSurfaceBounds({
      x: rect.x,
      y: rect.y,
      width: Math.max(1, rect.width),
      height: Math.max(1, rect.height),
      visible: false,
    });
  }, [setSurfaceBounds, overlayOpen]);

  const clearData = async () => {
    if (!workspaceId) return;
    setOverlayOpen(true);
    const ok = await confirm({
      title: t("このブラウザの閲覧データを削除しますか？"),
      description: t(
        "Cookie・localStorage・キャッシュが消えます。開いているタブも閉じます。",
      ),
      confirmLabel: t("削除"),
      tone: "danger",
    });
    setOverlayOpen(false);
    if (!ok) return;
    try {
      await browser.clearData();
    } catch {
      // The error is surfaced by the hook on the next info refresh.
    }
  };

  if (!workspaceId) {
    return (
      <PanelChrome
        full={full}
        onToggleFull={toggleSidePanelMode}
        onClose={dismissBrowserPanel}
      >
        <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-xs text-muted">
          <Globe size={18} className="text-faint" />
          <p className="m-0">{t("スレッドを開始するとブラウザを使えます。")}</p>
        </div>
      </PanelChrome>
    );
  }

  return (
    <div data-browser-pane className="flex h-full min-h-0 flex-col bg-bg">
      <BrowserTabStrip
        tabs={browser.tabs}
        loading={browser.navigating}
        full={full}
        onSwitch={(id) => void browser.switchTab(id)}
        onClose={(id) => void browser.closeTab(id)}
        onNew={() => void browser.newTab()}
        onToggleFull={toggleSidePanelMode}
        onClosePanel={dismissBrowserPanel}
      />
      <BrowserToolbar
        browser={browser}
        workspaceId={workspaceId}
        agentBusy={browserToolRunning}
        autoOpen={autoOpen}
        full={full}
        docked={docked}
        autoFocusAddress={openedBy === "user"}
        onToggleAutoOpen={() => setBrowserAutoOpen(!autoOpen)}
        onToggleFull={toggleSidePanelMode}
        onToggleDocked={toggleDocked}
        onResetRatio={() => setRightPanelRatio(DEFAULT_BROWSER_PANEL_RATIO)}
        onClearData={() => void clearData()}
        onOpenSettings={() => openSettings("browser")}
        onClose={dismissBrowserPanel}
        onMenuOpenChange={setOverlayOpen}
      />
      <BrowserSurface setBounds={setSurfaceBounds} visible={!overlayOpen} />
      {browser.error && (
        <p className="m-0 border-t border-line bg-bg px-2 py-1 text-xs text-danger">
          {browser.error}
        </p>
      )}
      {dialog}
    </div>
  );
}
