import { Bot, MessageSquare } from "lucide-react";
import { useUi, type Surface } from "@/lib/ui";
import { t } from "@/lib/i18n";
import { recordUxEvent } from "@/lib/uxEvents";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";

interface SurfaceMeta {
  id: Surface;
  label: string;
  shortLabel: string;
  hint: string;
  Icon: typeof MessageSquare;
}

const SURFACES: SurfaceMeta[] = [
  {
    id: "chat",
    label: "チャット",
    shortLabel: "チャット",
    hint: "作成、学習、探索。プロジェクトなしで使えます。",
    Icon: MessageSquare,
  },
  {
    id: "desktop-agent",
    label: "コーディング",
    shortLabel: "コード",
    hint: "ファイル編集とターミナルを使うワークスペース。",
    Icon: Bot,
  },
];

/**
 * The two top-level workspaces.
 *
 * This used to be a dropdown whose trigger showed only the active surface: the
 * other side was invisible until it was opened, and switching made the whole
 * thread list change with no explanation. Keeping both options on screen makes
 * the split explicit — the history is still there, the panel is just filtered.
 */
export function SurfaceSwitcher() {
  const surface = useUi((s) => s.surface);
  const setSurface = useUi((s) => s.setSurface);

  const choose = (next: Surface) => {
    if (next === surface) return;
    recordUxEvent("surface_switched", { from: surface, to: next });
    setSurface(next);
  };

  return (
    <Segmented aria-label={t("ワークスペースを切り替え")}>
      {SURFACES.map((s) => (
        <SegmentedOption
          key={s.id}
          value={s.id}
          label={t(s.hint)}
          selected={s.id === surface}
          onSelect={choose}
          data-testid={`surface-switcher-item-${s.id}`}
        >
          <s.Icon size={13} aria-hidden="true" />
          <span className="hidden md:inline">{t(s.label)}</span>
          <span className="md:hidden">{t(s.shortLabel)}</span>
        </SegmentedOption>
      ))}
    </Segmented>
  );
}
