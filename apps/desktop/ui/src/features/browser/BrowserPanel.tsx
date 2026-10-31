import { useCallback, useEffect, useRef } from "react";
import {
  Download,
  Globe,
  Maximize2,
  Minimize2,
  Settings2,
  X,
} from "lucide-react";
import { t } from "@/lib/i18n";
import { errorMessage } from "@/lib/errors";
import { useUi } from "@/lib/ui";
import { useRunEvents } from "@/lib/runStore";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { BrowserTabStrip } from "./BrowserTabStrip";
import { BrowserToolbar } from "./BrowserToolbar";
import { BrowserViewport } from "./BrowserViewport";
import { useBrowserStatus } from "./useBrowserStatus";
import { useLiveFrame } from "./useLiveFrame";

/** Minimal chrome for the states that cannot show the page toolbar (missing
 *  workspace, loading, no Chromium). Full view has no `RightPanel` header, so
 *  each of those states still needs its own close/full-view controls. */
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
 * The thread side-panel browser. Codex-like layout: toolbar, page tabs, live
 * viewport. Chromium management stays in Settings ▸ Browser; this pane only
 * surfaces an install CTA when the backend has no executable.
 */
export function BrowserPanel() {
  const workspaceId = useUi((s) => s.workspaceId);
  const openedBy = useUi((s) => s.browserOpenedBy);
  const autoOpen = useUi((s) => s.browserAutoOpen);
  const setBrowserAutoOpen = useUi((s) => s.setBrowserAutoOpen);
  const full = useUi((s) => s.sidePanelMode === "full");
  const toggleSidePanelMode = useUi((s) => s.toggleSidePanelMode);
  const dismissBrowserPanel = useUi((s) => s.dismissBrowserPanel);
  const openSettings = useUi((s) => s.openSettings);
  const conversationId = useUi((s) => s.conversationId);

  const browserToolRunning = useRunEvents((s) => {
    if (!conversationId) return false;
    return (
      s.byConversation[conversationId]?.timeline.tools.some(
        (tool) => tool.name === "browser" && tool.status === "running",
      ) ?? false
    );
  });

  const reportError = useCallback((error: unknown) => {
    console.warn("browser panel:", errorMessage(error));
  }, []);
  const { status, loading, installBusy, installActive, startInstall, refresh } =
    useBrowserStatus(reportError);
  const live = useLiveFrame(workspaceId, {
    // Only start once we know Chromium exists: an auto-opened pane during a
    // missing-install state should show the install CTA, not launch a doomed
    // screencast.
    autoStart: openedBy === "agent" && Boolean(status?.executable),
  });

  const liveRef = useRef(live);
  liveRef.current = live;
  useEffect(() => {
    const newTab = () => void liveRef.current.newTab();
    const reload = () => void liveRef.current.reload();
    window.addEventListener("mix:browser-new-tab", newTab);
    window.addEventListener("mix:browser-reload", reload);
    return () => {
      window.removeEventListener("mix:browser-new-tab", newTab);
      window.removeEventListener("mix:browser-reload", reload);
    };
  }, []);

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

  if (loading && !status) {
    return (
      <PanelChrome
        full={full}
        onToggleFull={toggleSidePanelMode}
        onClose={dismissBrowserPanel}
      >
        <div className="flex flex-1 items-center justify-center">
          <Spinner size={14} label={t("読み込み中…")} />
        </div>
      </PanelChrome>
    );
  }

  if (status && !status.executable) {
    return (
      <PanelChrome
        full={full}
        onToggleFull={toggleSidePanelMode}
        onClose={dismissBrowserPanel}
      >
        <div className="flex flex-1 flex-col gap-3 overflow-auto p-4">
          <p className="m-0 text-xs text-muted">
            {t(
              "ブラウザ機能を使うにはChromiumをインストールしてください。約200MBをダウンロードします。",
            )}
          </p>
          {installActive ? (
            <div className="flex flex-col gap-1.5">
              <div className="h-1.5 overflow-hidden rounded-full bg-inset">
                <div
                  className="h-full rounded-full bg-brand transition-[width]"
                  style={{ width: `${status.install.progress ?? 0}%` }}
                />
              </div>
              <span className="text-2xs text-muted">
                {status.install.stage ?? t("インストール中…")}
              </span>
            </div>
          ) : (
            <Button
              size="sm"
              disabled={installBusy}
              onClick={() => void startInstall(false)}
            >
              <Download size={12} /> {t("Chromium をインストール")}
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="self-start"
            onClick={() => openSettings("browser")}
          >
            <Settings2 size={12} /> {t("ブラウザ設定を開く")}
          </Button>
          {status.install.failure && (
            <p className="m-0 text-xs text-danger">{status.install.failure}</p>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="self-start"
            onClick={() => void refresh()}
          >
            {t("状態を更新")}
          </Button>
        </div>
      </PanelChrome>
    );
  }

  return (
    <div data-browser-pane className="flex h-full min-h-0 flex-col">
      <BrowserToolbar
        live={live}
        agentBusy={browserToolRunning}
        autoOpen={autoOpen}
        full={full}
        autoFocusAddress={openedBy === "user"}
        onToggleAutoOpen={() => setBrowserAutoOpen(!autoOpen)}
        onToggleFull={toggleSidePanelMode}
        onOpenSettings={() => openSettings("browser")}
        onClose={dismissBrowserPanel}
      />
      <BrowserTabStrip
        tabs={live.tabs}
        onSwitch={(id) => void live.switchTab(id)}
        onClose={(id) => void live.closeTab(id)}
        onNew={() => void live.newTab()}
      />
      <div className="flex min-h-0 flex-1 flex-col p-2">
        <BrowserViewport live={live} fill hideToolbar />
      </div>
    </div>
  );
}
