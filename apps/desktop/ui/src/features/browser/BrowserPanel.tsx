import { useEffect, useState } from "react";
import { Globe } from "lucide-react";
import { t } from "@/lib/i18n";
import { DEFAULT_BROWSER_PANEL_RATIO, useUi } from "@/lib/ui";
import { useRunEvents } from "@/lib/runStore";
import { useConfirm } from "@/components/ui/dialog";
import { BrowserSurface } from "./BrowserSurface";
import { BrowserTabStrip } from "./BrowserTabStrip";
import { BrowserToolbar } from "./BrowserToolbar";
import { useWebviewBrowser } from "./useWebviewBrowser";

/**
 * The thread side-panel browser. The page is a native Tauri child WebView;
 * React owns only the tab strip, the omnibox and the surface rectangle.
 */
export function BrowserPanel() {
  const workspaceId = useUi((s) => s.workspaceId);
  const openedBy = useUi((s) => s.browserOpenedBy);
  const autoOpen = useUi((s) => s.browserAutoOpen);
  const setBrowserAutoOpen = useUi((s) => s.setBrowserAutoOpen);
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
      <div className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center text-xs text-muted">
        <Globe size={18} className="text-faint" />
        <p className="m-0">{t("スレッドを開始するとブラウザを使えます。")}</p>
      </div>
    );
  }

  return (
    <div data-browser-pane className="flex h-full min-h-0 flex-col bg-bg">
      <BrowserTabStrip
        tabs={browser.tabs}
        loading={browser.navigating}
        onSwitch={(id) => void browser.switchTab(id)}
        onClose={(id) => void browser.closeTab(id)}
        onNew={() => void browser.newTab()}
      />
      <BrowserToolbar
        browser={browser}
        workspaceId={workspaceId}
        agentBusy={browserToolRunning}
        autoOpen={autoOpen}
        autoFocusAddress={openedBy === "user"}
        onToggleAutoOpen={() => setBrowserAutoOpen(!autoOpen)}
        onResetRatio={() => setRightPanelRatio(DEFAULT_BROWSER_PANEL_RATIO)}
        onClearData={() => void clearData()}
        onOpenSettings={() => openSettings("browser")}
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
