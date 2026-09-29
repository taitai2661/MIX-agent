import { Bot, Check, ChevronDown, MessageSquare } from "lucide-react";
import { useUi, type Surface } from "@/lib/ui";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem,
  DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

interface SurfaceMeta {
  id: Surface;
  label: string;
  hint: string;
  Icon: typeof MessageSquare;
}

const SURFACES: SurfaceMeta[] = [
  { id: "chat", label: "chat", hint: "作成、学習、探索", Icon: MessageSquare },
  { id: "desktop-agent", label: "desktop-agent", hint: "ビルド、デバッグ、リリース", Icon: Bot },
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
          aria-label={t("サーフェスを切り替え")}
          title={t("サーフェスを切り替え")}
          data-testid="surface-switcher-trigger"
        >
          <active.Icon size={14} className="text-faint" />
          <span>{active.label}</span>
          <ChevronDown size={13} className="text-faint" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" sideOffset={6} className="min-w-56">
        <DropdownMenuLabel>{t("サーフェスを切り替え")}</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {SURFACES.map((s) => {
          const selected = s.id === surface;
          return (
            <DropdownMenuItem
              key={s.id}
              data-testid={`surface-switcher-item-${s.id}`}
              data-selected={selected}
              onSelect={() => setSurface(s.id)}
              className={cn(
                "flex items-start gap-2.5 py-2",
                selected && "bg-active",
              )}
            >
              <s.Icon size={15} className="mt-0.5 shrink-0 text-faint" />
              <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="text-sm font-medium text-fg">{t(s.label)}</span>
                <span className="text-2xs leading-tight text-muted">{t(s.hint)}</span>
              </span>
              {selected && (
                <Check
                  size={14}
                  className="mt-0.5 shrink-0 text-fg"
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
