import { Activity, GitBranch, X } from "lucide-react";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { ActivityPanel } from "@/features/chat/ActivityPanel";
import { DiffPanel } from "@/features/chat/DiffView";
import { ChangesPanel, FilePreviewPanel } from "@/features/files/ChangesPanel";
import { Button } from "@/components/ui/button";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";
import { ResizableHandle } from "./ResizableHandle";
import { panelStyle, panelWidth } from "./usePanelCap";
import { usePanelWidth } from "./usePanelWidth";

export function RightPanel({ cap = null }: { cap?: number | null }) {
  const rightPanel = useUi((s) => s.rightPanel);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const { width, setWidth, commitWidth } = usePanelWidth(
    "mix-agent-right-panel-width",
    380,
    300,
    720,
  );

  if (rightPanel === "none") return null;

  const tabbed = rightPanel === "activity" || rightPanel === "changes";

  return (
    <>
      <ResizableHandle
        value={panelWidth(width, cap)}
        min={300}
        max={720}
        direction={-1}
        label={t("サイドパネルの幅")}
        onChange={setWidth}
        onCommit={commitWidth}
      />
      <aside
        style={panelStyle(width, cap)}
        data-side-panel=""
        aria-label={t("サイドパネル")}
        className="flex shrink-0 flex-col overflow-hidden border-l border-line bg-bg"
      >
        {tabbed && (
          <div className="flex h-titlebar shrink-0 items-center gap-1.5 border-b border-line bg-surface/80 px-2 backdrop-blur">
            <Segmented
              aria-label={t("サイドパネル")}
              className="min-w-0 flex-1"
            >
              <SegmentedOption
                value="activity"
                label={t("このRunのアクティビティ")}
                selected={rightPanel === "activity"}
                onSelect={() => setRightPanel("activity")}
              >
                <Activity size={12} aria-hidden="true" /> {t("アクティビティ")}
              </SegmentedOption>
              <SegmentedOption
                value="changes"
                label={t("変更")}
                selected={rightPanel === "changes"}
                onSelect={() => setRightPanel("changes")}
              >
                <GitBranch size={12} aria-hidden="true" /> {t("変更")}
              </SegmentedOption>
            </Segmented>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => setRightPanel("none")}
              title={t("閉じる")}
              aria-label={t("閉じる")}
            >
              <X size={13} />
            </Button>
          </div>
        )}
        {rightPanel === "diff" && <DiffPanel />}
        {rightPanel === "changes" && <ChangesPanel embedded />}
        {rightPanel === "file" && <FilePreviewPanel />}
        {rightPanel === "activity" && <ActivityPanel embedded />}
      </aside>
    </>
  );
}
