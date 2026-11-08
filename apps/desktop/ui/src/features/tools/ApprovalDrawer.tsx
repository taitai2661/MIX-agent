import { useEffect, useRef, type Ref } from "react";
import { ShieldAlert } from "lucide-react";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { usePendingApprovals } from "@/lib/queries";
import { PanelHeader } from "@/components/layout";
import { Badge } from "@/components/ui/badge";
import { ApprovalsPane } from "./ApprovalsPane";

/**
 * Approval inbox that stays inside the conversation.
 *
 * The title-bar indicator used to navigate to Settings ▸ 承認, which replaced
 * the transcript with the settings pane at exactly the moment a run was
 * blocked. This drawer is non-modal: it overlays the right edge, leaves the
 * composer usable, and closes on Escape or an outside click.
 */
export function ApprovalDrawer() {
  const open = useUi((s) => s.approvalsOpen);
  const setOpen = useUi((s) => s.setApprovalsOpen);
  const pending = usePendingApprovals();
  const asideRef = useRef<HTMLElement | null>(null);
  const count = pending.data?.length ?? 0;

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      const aside = asideRef.current;
      if (aside && !aside.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [open, setOpen]);

  if (!open) return null;

  return (
    <aside
      ref={asideRef as Ref<HTMLElement>}
      role="dialog"
      aria-modal="false"
      aria-label={t("承認待ち")}
      className="absolute top-titlebar bottom-0 right-0 z-40 flex w-[min(440px,92vw)] flex-col overflow-hidden border-l border-line bg-bg shadow-pop animate-fade"
    >
      <PanelHeader
        icon={<ShieldAlert size={14} className="text-warning" />}
        title={t("承認待ち")}
        onClose={() => setOpen(false)}
        titleClass="text-sm font-semibold"
        trailing={
          count > 0 ? (
            <Badge tone="warning" size="sm" className="font-mono">
              {count}
            </Badge>
          ) : undefined
        }
      />
      <div className="stable-scroll min-h-0 flex-1 overflow-y-auto p-4">
        <ApprovalsPane location="drawer" />
      </div>
    </aside>
  );
}
