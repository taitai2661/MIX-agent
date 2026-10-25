import { useEffect, useRef, type Ref } from "react";
import {
  Activity,
  Columns2,
  GitBranch,
  Maximize2,
  Minimize2,
  PanelRight,
  X,
} from "lucide-react";
import { t } from "@/lib/i18n";
import { useUi, WIDE_RIGHT_PANEL_WIDTH } from "@/lib/ui";
import { ActivityPanel } from "@/features/chat/ActivityPanel";
import { DiffPanel } from "@/features/chat/DiffView";
import { ChangesPanel, FilePreviewPanel } from "@/features/files/ChangesPanel";
import { Button } from "@/components/ui/button";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";
import { panelStyle } from "./usePanelCap";

export function RightPanel({ cap = null }: { cap?: number | null }) {
  const rightPanel = useUi((s) => s.rightPanel);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const rightPanelWidth = useUi((s) => s.rightPanelWidth);
  const toggleRightPanelWidth = useUi((s) => s.toggleRightPanelWidth);
  const isDocked = useUi((s) => s.rightPanelDocked);
  const toggleDocked = useUi((s) => s.toggleRightPanelDocked);
  const isWide = rightPanelWidth >= WIDE_RIGHT_PANEL_WIDTH;
  const asideRef = useRef<HTMLElement | null>(null);
  const returnFocusRef = useRef<Element | null>(null);

  // Overlay drawer: Escape closes it. Focus stays where it was — the drawer
  // never steals it — so there is no trap to manage.
  useEffect(() => {
    if (rightPanel === "none") return;
    // Remember where focus was so closing returns it there.
    returnFocusRef.current = document.activeElement;
    const aside = asideRef.current;
    // Move focus into the drawer on open for keyboard users.
    const first = aside?.querySelector<HTMLElement>(
      "button, [href], input, select, textarea, [tabindex]:not([tabindex='-1'])",
    );
    (first ?? aside)?.focus?.();
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // Do not steal Escape from an open modal dialog / menu above us.
        if (document.querySelector("[role='dialog'][data-modal='true']"))
          return;
        e.preventDefault();
        setRightPanel("none");
      }
      if (e.key === "Tab" && aside) {
        // Minimal focus trap: keep Tab cycling inside the drawer.
        const items = Array.from(
          aside.querySelectorAll<HTMLElement>(
            "button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex='-1'])",
          ),
        ).filter((el) => el.offsetParent !== null);
        if (items.length === 0) return;
        const firstItem = items[0];
        const lastItem = items[items.length - 1];
        if (e.shiftKey && document.activeElement === firstItem) {
          e.preventDefault();
          lastItem.focus();
        } else if (!e.shiftKey && document.activeElement === lastItem) {
          e.preventDefault();
          firstItem.focus();
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      // Return focus to the opener on close.
      (returnFocusRef.current as HTMLElement | null)?.focus?.();
    };
  }, [rightPanel, setRightPanel]);

  if (rightPanel === "none") return null;

  const tabbed = rightPanel === "activity" || rightPanel === "changes";

  // Can be rendered either as a docked flex sibling (split-view) or as an
  // overlay drawer depending on `isDocked`.
  return (
    <aside
      ref={asideRef as Ref<HTMLElement>}
      tabIndex={-1}
      data-side-panel=""
      style={panelStyle(rightPanelWidth, cap)}
      role={isDocked ? "region" : "dialog"}
      aria-modal="false"
      aria-label={t("サイドパネル")}
      className={
        isDocked
          ? "relative z-10 flex shrink-0 flex-col overflow-hidden border-l border-line bg-bg animate-fade"
          : "absolute inset-y-0 right-0 z-30 flex max-w-[85vw] flex-col overflow-hidden border-l border-line bg-bg shadow-pop animate-fade"
      }
    >
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
              value="changes"
              label={t("変更")}
              icon={<GitBranch size={13} aria-hidden="true" />}
              selected={rightPanel === "changes"}
              onSelect={() => setRightPanel("changes")}
            >
              {t("変更")}
            </SegmentedOption>
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
            onClick={() => setRightPanel("none")}
            title={t("閉じる")}
            aria-label={t("閉じる")}
          >
            <X size={14} />
          </Button>
        </div>
      )}
      {rightPanel === "diff" && <DiffPanel />}
      {rightPanel === "changes" && <ChangesPanel embedded />}
      {rightPanel === "file" && <FilePreviewPanel />}
      {rightPanel === "activity" && <ActivityPanel embedded />}
    </aside>
  );
}
