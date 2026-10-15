import { forwardRef } from "react";
import { cva } from "class-variance-authority";
import { cn } from "@/lib/cn";

/**
 * Segmented control — a small set of mutually exclusive options, exactly one
 * of which is active.
 *
 * Eight groups in this app used to reimplement it (the sidebar's thread filter,
 * the onboarding provider tabs, the skills import source, the staged/unstaged
 * switch, the theme and language pickers, the cron presets, the browser action
 * picker, the wizard's step rail) across three container treatments, three
 * heights and two outer radii.
 *
 * It is a `radiogroup` rather than a tablist: these switch a *filter or a
 * value*, they do not reveal a panel, and a tablist without a `tabpanel` tells
 * a screen reader there is content it cannot reach.
 *
 * The options are real buttons, so each is its own tab stop rather than one
 * group with arrow-key navigation. That is a deliberate trade: four of the five
 * uses sit next to a search box or a form the user tabs through anyway, and
 * roving focus would make the other four harder to reach with a keyboard.
 */
/**
 * One height, no `size` variant. The four groups that use this were previously
 * spread across two heights; the reason to add a variant back is that a group
 * genuinely has to differ, not that one of them looked nicer.
 */
const segmentedVariants = cva(
  "inline-flex h-row items-center gap-0.5 rounded-md border border-line/60 bg-inset p-0.5",
);

export const Segmented = forwardRef<
  HTMLDivElement,
  React.ComponentProps<"div">
>(function Segmented({ className, ...props }, ref) {
  return (
    <div
      ref={ref}
      role="radiogroup"
      className={cn(segmentedVariants(), className)}
      {...props}
    />
  );
});

export interface SegmentedOption<T extends string> {
  value: T;
  /** Visible content. */
  children: React.ReactNode;
  /** Optional leading icon. */
  icon?: React.ReactNode;
  /** Optional trailing count / metadata, rendered after the label. */
  trailing?: React.ReactNode;
  /** Accessible + tooltip text. Use when `children` is an icon or an
   *  abbreviation that would not read well aloud. */
  label?: string;
  disabled?: boolean;
}

export function SegmentedOption<T extends string>({
  value,
  children,
  icon,
  trailing,
  label,
  disabled = false,
  selected,
  onSelect,
  ...props
}: {
  value: T;
  selected: boolean;
  onSelect: (value: T) => void;
} & SegmentedOption<T> &
  Omit<React.ComponentProps<"button">, "value" | "onSelect" | "aria-checked">) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={() => onSelect(value)}
      className={cn(
        "inline-flex h-full min-w-0 flex-1 cursor-pointer items-center justify-center gap-1.5 rounded-sm px-2.5",
        "text-xs font-medium whitespace-nowrap transition-colors duration-100",
        selected
          ? "bg-surface text-fg shadow-xs"
          : "text-muted hover:text-fg hover:bg-hover/60",
      )}
      {...props}
    >
      {icon && (
        <span className="inline-flex shrink-0 items-center justify-center text-current" aria-hidden="true">
          {icon}
        </span>
      )}
      {/* `min-w-0` on the label is what lets it reach the ellipsis: without
          it the label keeps its `min-content` width, the option cannot shrink,
          and a group of three Japanese labels pushed a whole side panel past
          the window edge. */}
      <span className="inline-flex min-w-0 items-center justify-center gap-1.5 truncate [&>svg]:shrink-0 [&>svg]:inline-block">
        {children}
      </span>
      {trailing != null && (
        <span className="shrink-0 font-mono text-2xs">{trailing}</span>
      )}
    </button>
  );
}
