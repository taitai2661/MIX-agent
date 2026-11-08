import { useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import {
  Activity,
  Columns2,
  Expand,
  Gauge,
  GitBranch,
  Globe,
  Maximize2,
  Minimize2,
  MoreHorizontal,
  PanelRight,
  Shrink,
  X,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { useUi, WIDE_RIGHT_PANEL_WIDTH } from "@/lib/ui";
import {
  RIGHT_PANEL_TABS,
  isPanelAvailable,
  isRightPanelTab,
  type RightPanelTab,
} from "@/lib/panelTabs";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

const TAB_ICONS: Record<RightPanelTab, LucideIcon> = {
  activity: Activity,
  usage: Gauge,
  changes: GitBranch,
  browser: Globe,
};

/**
 * One tab strip for every right-panel destination.
 *
 * The browser used to own a second, chrome-like header (page tabs plus its
 * own full/close buttons) while activity/usage/changes had a segmented
 * control. That made "browser" feel like a different pane instead of another
 * tab. This strip is the single switcher; browser page tabs now live below it,
 * inside the browser body.
 */
export function RightPanelHeader({ compact = false }: { compact?: boolean }) {
  const rightPanel = useUi((s) => s.rightPanel);
  const surface = useUi((s) => s.surface);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const openBrowserPanel = useUi((s) => s.openBrowserPanel);
  const closeRightPanel = useUi((s) => s.closeRightPanel);
  const sidePanelMode = useUi((s) => s.sidePanelMode);
  const toggleSidePanelMode = useUi((s) => s.toggleSidePanelMode);
  const toggleDocked = useUi((s) => s.toggleRightPanelDocked);
  const isDocked = useUi((s) => s.rightPanelDocked);
  const rightPanelWidth = useUi((s) => s.rightPanelWidth);
  const toggleRightPanelWidth = useUi((s) => s.toggleRightPanelWidth);

  const isAgentSurface = surface === "desktop-agent";
  const isFull = sidePanelMode === "full";
  const isWide = rightPanelWidth >= WIDE_RIGHT_PANEL_WIDTH;
  const active = isRightPanelTab(rightPanel) ? rightPanel : null;

  const selectTab = (tab: RightPanelTab) => {
    // Full view belongs to the panel, not to a single tab: switching tabs
    // while expanded keeps the centre column expanded.
    if (tab === "browser") {
      if (rightPanel !== "browser") openBrowserPanel("user");
      return;
    }
    setRightPanel(tab);
  };

  return (
    <header
      data-right-panel-chrome
      className="flex h-titlebar shrink-0 items-center gap-1 border-b border-line bg-bg/85 px-2 backdrop-blur-md"
    >
      <RightPanelTabs
        active={active}
        isAgentSurface={isAgentSurface}
        compact={compact}
        onSelect={selectTab}
      />
      <div className="flex shrink-0 items-center gap-0.5">
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={toggleSidePanelMode}
          title={isFull ? t("スプリット表示に戻す") : t("フル表示")}
          aria-label={isFull ? t("スプリット表示に戻す") : t("フル表示")}
          aria-pressed={isFull}
        >
          {isFull ? <Shrink size={13} /> : <Expand size={13} />}
        </Button>
        {!isFull && (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="icon-sm"
                title={t("パネルの表示設定")}
                aria-label={t("パネルの表示設定")}
              >
                <MoreHorizontal size={15} />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-52">
              <DropdownMenuLabel>{t("パネルの表示設定")}</DropdownMenuLabel>
              <DropdownMenuItem onSelect={toggleRightPanelWidth}>
                {isWide ? (
                  <Minimize2 size={13} className="text-muted" />
                ) : (
                  <Maximize2 size={13} className="text-muted" />
                )}
                <span className="flex-1">
                  {isWide ? t("標準幅に戻す") : t("パネル幅を拡大")}
                </span>
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={toggleDocked}>
                {isDocked ? (
                  <PanelRight size={13} className="text-brand" />
                ) : (
                  <Columns2 size={13} className="text-muted" />
                )}
                <span className="flex-1">
                  {isDocked
                    ? t("オーバーレイ表示に切り替え")
                    : t("右側に固定表示")}
                </span>
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        )}
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={closeRightPanel}
          title={t("閉じる")}
          aria-label={t("閉じる")}
        >
          <X size={14} />
        </Button>
      </div>
    </header>
  );
}

interface RightPanelTabsProps {
  active: RightPanelTab | null;
  isAgentSurface: boolean;
  compact: boolean;
  onSelect: (tab: RightPanelTab) => void;
}

/**
 * The tab strip itself, with a pill that slides and resizes to the selected
 * tab. When the panel is narrow, only the selected tab keeps its label; the
 * others collapse to icons, so the bar deforms instead of overflowing.
 *
 * Keyboard model follows the WAI-ARIA tabs pattern: arrows move between tabs
 * (wrapping at the ends), Home/End jump to the first/last, and selection
 * follows focus. Only the selected tab is in the Tab order.
 */
function RightPanelTabs({
  active,
  isAgentSurface,
  compact,
  onSelect,
}: RightPanelTabsProps) {
  const listRef = useRef<HTMLDivElement | null>(null);
  const [indicator, setIndicator] = useState<{
    left: number;
    width: number;
  } | null>(null);
  const visibleTabs = RIGHT_PANEL_TABS.filter((tab) =>
    isPanelAvailable(tab.value, isAgentSurface),
  );
  const firstTab = visibleTabs[0]?.value ?? null;

  useLayoutEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const measure = () => {
      const selected = list.querySelector<HTMLElement>(
        '[role="tab"][aria-selected="true"]',
      );
      if (!selected) {
        setIndicator(null);
        return;
      }
      const next = {
        left: selected.offsetLeft,
        width: selected.offsetWidth,
      };
      setIndicator((current) =>
        current?.left === next.left && current.width === next.width
          ? current
          : next,
      );
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(list);
    return () => observer.disconnect();
  }, [active, compact, isAgentSurface]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const handled = ["ArrowLeft", "ArrowRight", "Home", "End"] as const;
    if (!handled.includes(event.key as (typeof handled)[number])) return;
    const buttons = Array.from(
      listRef.current?.querySelectorAll<HTMLButtonElement>('[role="tab"]') ??
        [],
    );
    if (buttons.length === 0) return;
    const focused = buttons.findIndex(
      (button) => button === document.activeElement,
    );
    const selected = buttons.findIndex(
      (button) => button.dataset.tab === active,
    );
    const index = focused >= 0 ? focused : Math.max(0, selected);
    let next = index;
    if (event.key === "ArrowLeft") {
      next = index <= 0 ? buttons.length - 1 : index - 1;
    } else if (event.key === "ArrowRight") {
      next = index >= buttons.length - 1 ? 0 : index + 1;
    } else if (event.key === "Home") {
      next = 0;
    } else {
      next = buttons.length - 1;
    }
    event.preventDefault();
    const target = buttons[next];
    target.focus();
    const tab = target.dataset.tab;
    if (tab && isRightPanelTab(tab)) onSelect(tab);
  };

  return (
    <div
      ref={listRef}
      role="tablist"
      aria-label={t("パネル切り替え")}
      aria-orientation="horizontal"
      tabIndex={-1}
      onKeyDown={onKeyDown}
      className="relative flex h-row min-w-0 flex-1 items-center gap-0.5 overflow-x-auto rounded-lg border border-line/60 bg-inset p-0.5 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
    >
      {indicator && (
        <span
          aria-hidden="true"
          className="pointer-events-none absolute top-0.5 bottom-0.5 left-0 rounded-md bg-surface shadow-xs transition-[transform,width] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none"
          style={{
            transform: `translateX(${indicator.left}px)`,
            width: indicator.width,
          }}
        />
      )}
      {visibleTabs.map((tab) => {
        const Icon = TAB_ICONS[tab.value];
        const selected = active === tab.value;
        const showLabel = !compact || selected;
        const label = t(tab.label);
        return (
          <button
            key={tab.value}
            type="button"
            role="tab"
            id={`right-panel-tab-${tab.value}`}
            data-tab={tab.value}
            tabIndex={
              selected || (active === null && tab.value === firstTab) ? 0 : -1
            }
            aria-selected={selected}
            aria-controls="right-panel-body"
            aria-label={label}
            title={label}
            onClick={() => onSelect(tab.value)}
            className={cn(
              "relative z-10 inline-flex h-full shrink-0 cursor-pointer items-center justify-center gap-1.5 rounded-md px-2.5",
              "text-xs font-medium whitespace-nowrap transition-colors duration-100 outline-none",
              "focus-visible:ring-2 focus-visible:ring-brand/50",
              selected
                ? "text-fg"
                : "text-muted hover:bg-hover/60 hover:text-fg",
              compact && "px-2",
            )}
          >
            <Icon
              size={13}
              className={cn("shrink-0", selected && "text-brand")}
            />
            {showLabel && <span className="min-w-0 truncate">{label}</span>}
          </button>
        );
      })}
    </div>
  );
}
