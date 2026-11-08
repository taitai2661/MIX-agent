import { Suspense, lazy } from "react";
import { t } from "@/lib/i18n";
import type { RightPanel } from "@/lib/ui";
import { isPanelAvailable, isRightPanelTab } from "@/lib/panelTabs";
import { ActivityPanel } from "@/features/chat/ActivityPanel";
import { UsagePanel } from "@/features/chat/UsagePanel";
import { DiffPanel } from "@/features/chat/DiffView";
import { ChangesPanel, FilePreviewPanel } from "@/features/files/ChangesPanel";
import { Spinner } from "@/components/ui/spinner";

const BrowserPanel = lazy(() =>
  import("@/features/browser/BrowserPanel").then((m) => ({
    default: m.BrowserPanel,
  })),
);

/**
 * Shared body renderer so the panel looks and behaves the same whether it is
 * docked on the right, floating as an overlay, or morphed into the centre
 * column by the full-view toggle.
 */
export function RightPanelBody({
  panel,
  isAgentSurface,
}: {
  panel: RightPanel;
  isAgentSurface: boolean;
}) {
  // A panel restored from storage may belong to the other surface (the
  // browser on the chat surface, for example). `RightPanel` redirects it in an
  // effect, but rendering nothing for a frame leaves a blank panel; fall back
  // to the usage panel, which is the surface-agnostic destination.
  const resolved =
    isRightPanelTab(panel) && !isPanelAvailable(panel, isAgentSurface)
      ? "usage"
      : panel;
  const tabbed = isRightPanelTab(resolved);

  return (
    <div
      id="right-panel-body"
      role={tabbed ? "tabpanel" : undefined}
      aria-labelledby={tabbed ? `right-panel-tab-${resolved}` : undefined}
      className="flex min-h-0 flex-1 flex-col"
    >
      {resolved === "diff" && isAgentSurface && <DiffPanel />}
      {resolved === "changes" && isAgentSurface && <ChangesPanel embedded />}
      {resolved === "file" && isAgentSurface && <FilePreviewPanel />}
      {resolved === "activity" && <ActivityPanel embedded />}
      {resolved === "usage" && <UsagePanel embedded />}
      {resolved === "browser" && isAgentSurface && (
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
    </div>
  );
}
