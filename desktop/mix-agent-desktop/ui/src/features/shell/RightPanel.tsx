import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { ActivityPanel } from "@/features/chat/ActivityPanel";
import { DiffPanel } from "@/features/chat/DiffView";
import { ChangesPanel, FilePreviewPanel } from "@/features/files/ChangesPanel";
import { ResizableHandle } from "./ResizableHandle";
import { usePanelWidth } from "./usePanelWidth";

export function RightPanel() {
  const rightPanel = useUi((s) => s.rightPanel);
  const { width, setWidth, commitWidth } = usePanelWidth(
    "mix-agent-right-panel-width", 360, 280, 720,
  );

  if (rightPanel === "none") return null;

  return (
    <>
      <ResizableHandle
        value={width}
        min={280}
        max={720}
        direction={-1}
        label={t("サイドパネルの幅")}
        onChange={setWidth}
        onCommit={commitWidth}
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
