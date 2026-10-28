import { Bot, Check, ChevronDown, MessageSquare } from "lucide-react";
import { useUi, type Surface } from "@/lib/ui";
import { t } from "@/lib/i18n";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

interface SurfaceMeta {
  id: Surface;
  label: string;
  hint: string;
  Icon: typeof MessageSquare;
}

const SURFACES: SurfaceMeta[] = [
  {
    id: "chat",
    label: "チャット",
    hint: "作成、学習、探索",
    Icon: MessageSquare,
  },
  {
    id: "desktop-agent",
    label: "コーディング",
    hint: "ファイル編集とターミナルを使うワークスペース",
    Icon: Bot,
  },
];

export function SurfaceSwitcher() {
  const surface = useUi((s) => s.surface);
  const setSurface = useUi((s) => s.setSurface);
  const active = SURFACES.find((s) => s.id === surface) ?? SURFACES[0];

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="gap-1.5 px-2 font-semibold"
          aria-label={t("ワークスペースを切り替え")}
          title={t("ワークスペースを切り替え")}
          data-testid="surface-switcher-trigger"
        >
          <active.Icon size={15} className="text-faint" />
          <span>{active.label}</span>
          <ChevronDown size={12} className="text-faint" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" sideOffset={6} className="min-w-56">
        <DropdownMenuLabel>{t("ワークスペースを切り替え")}</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {SURFACES.map((s) => {
          const selected = s.id === surface;
          return (
            <DropdownMenuItem
              key={s.id}
              data-testid={`surface-switcher-item-${s.id}`}
              onSelect={() => setSurface(s.id)}
              description={t(s.hint)}
            >
              <s.Icon size={15} className="shrink-0 text-faint" />
              <span className="min-w-0 flex-1 truncate font-medium">
                {t(s.label)}
              </span>
              {selected && (
                <Check
                  size={14}
                  className="ms-auto shrink-0 text-brand"
                  aria-hidden="true"
                />
              )}
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
