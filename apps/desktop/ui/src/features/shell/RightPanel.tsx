import { useEffect, useRef, type Ref } from "react";
import { Activity, GitBranch, X } from "lucide-react";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { ActivityPanel } from "@/features/chat/ActivityPanel";
import { DiffPanel } from "@/features/chat/DiffView";
import { ChangesPanel, FilePreviewPanel } from "@/features/files/ChangesPanel";
import { Button } from "@/components/ui/button";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";
import { panelStyle } from "./usePanelCap";

/** Fixed Codex-like drawer width. */
const RIGHT_PANEL_WIDTH = 380;

export function RightPanel({ cap = null }: { cap?: number | null }) {
  const rightPanel = useUi((s) => s.rightPanel);
  const setRightPanel = useUi((s) => s.setRightPanel);
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
        if (document.querySelector("[role='dialog'][data-modal='true']")) return;
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

  // Overlay drawer, not a flex sibling: opening or closing it never changes
  // the centre column's width, so the transcript and composer stay exactly
  // where they are — the single biggest source of "unstable" layout shifts.
  // (The parent shell row is `relative`; this anchors to its right edge.)
  return (
    <aside
      ref={asideRef as Ref<HTMLElement>}
      tabIndex={-1}
      data-side-panel=""
      style={panelStyle(RIGHT_PANEL_WIDTH, cap)}
      role="dialog"
      aria-modal="false"
      aria-label={t("サイドパネル")}
      className="absolute inset-y-0 right-0 z-30 flex max-w-[85vw] flex-col overflow-hidden border-l border-line bg-bg shadow-pop animate-fade"
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
