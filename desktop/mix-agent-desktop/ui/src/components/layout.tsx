import type { ReactNode } from "react";
import { ArrowLeft, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { Button } from "./ui/button";

/** Scrollable page body for a top-level view (Projects, Agents, Schedules). */
export function Page({
  title,
  sub,
  actions,
  children,
  className,
}: {
  title?: string;
  sub?: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  const hasHeader = Boolean(title || sub || actions);
  return (
    <div className={cn("flex min-h-0 flex-1 flex-col", className)}>
      {hasHeader && (
        <header className="flex shrink-0 items-center border-b border-line bg-bg/80 px-6 py-4 backdrop-blur">
          <div className="mx-auto flex w-full max-w-content items-center gap-3">
            <div className="min-w-0 flex-1">
              {title && (
                <h1 className="truncate text-lg font-semibold tracking-tight text-fg">{title}</h1>
              )}
              {sub && <p className="mt-0.5 truncate text-xs text-muted">{sub}</p>}
            </div>
            {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
          </div>
        </header>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex w-full max-w-content flex-col gap-5 px-6 py-7">
          {children}
        </div>
      </div>
    </div>
  );
}

/**
 * The app's only card.
 *
 * A near-identical duplicate used to live in `components/ui/card.tsx` as
 * `Card`/`CardTitle`/`CardBody`, used by exactly one file. Its title slot was
 * forced on every consumer and the only place that wanted trailing content had
 * to bolt a `ml-auto` counter onto it — which is what the `action` slot is for.
 */
export function Card({
  title,
  icon,
  action,
  children,
  className,
  bodyClassName,
}: {
  title?: string;
  icon?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
}) {
  return (
    <section
      className={cn(
        "flex flex-col overflow-hidden rounded-xl border border-line bg-surface",
        className,
      )}
    >
      {(title || action) && (
        <div className="flex items-center gap-2 border-b border-line px-4 py-3">
          {icon && <span className="shrink-0 text-faint">{icon}</span>}
          {title && <h3 className="min-w-0 flex-1 text-sm font-semibold text-fg">{title}</h3>}
          {action}
        </div>
      )}
      <div className={cn("flex flex-col gap-3 p-4", bodyClassName)}>{children}</div>
    </section>
  );
}

/** Stacked group of cards, the standard body of a settings or list tab. */
export function Section({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("flex flex-col gap-4", className)}>{children}</div>;
}

/** Label + control + optional hint, with consistent vertical rhythm.
 *
 *  With a label, a real `<label>` wraps both the visible text and the control,
 *  so the control gets its accessible name implicitly and no `id`/`htmlFor`
 *  pair has to be threaded through sixteen call sites. The text is a `<span>`,
 *  **not** the `Label` component (Radix's `<label>`) — nesting one label inside
 *  another is invalid HTML and left the inner one pointing at nothing.
 *
 *  Without a label it is a plain `<div>`: several callers use this purely for
 *  the gap, and a wrapping label with no text would swallow the control's name
 *  rather than supply it. */
export function Field({
  label,
  hint,
  children,
  className,
}: {
  label?: string;
  hint?: string;
  children: ReactNode;
  className?: string;
}) {
  const control = label ? (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs font-medium text-muted select-none">{label}</span>
      {children}
    </label>
  ) : (
    children
  );
  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      {control}
      {hint && <span className="text-xs text-faint">{hint}</span>}
    </div>
  );
}

/** Horizontal run of controls with wrapping. */
export function Row({
  children,
  className,
  align = "center",
}: {
  children: ReactNode;
  className?: string;
  align?: "center" | "start" | "between" | "end";
}) {
  return (
    <div
      className={cn(
        "flex flex-wrap items-center gap-2",
        align === "start" && "items-start",
        align === "between" && "justify-between",
        align === "end" && "justify-end",
        className,
      )}
    >
      {children}
    </div>
  );
}

/** Bordered data table. `columns` are header cells; the row callback owns the body. */
export function DataTable<T>({
  columns,
  rows,
  rowKey,
  children,
}: {
  columns: ReactNode[];
  rows: readonly T[];
  rowKey: (row: T) => string;
  children: (row: T) => ReactNode;
}) {
  return (
    <div className="overflow-x-auto rounded-xl border border-line">
      <table className="w-full border-collapse text-xs">
        <thead>
          <tr className="border-b border-line bg-inset text-2xs text-muted">
            {columns.map((c, i) => (
              <th
                key={i}
                scope="col"
                className="px-3 py-2 text-start font-semibold whitespace-nowrap"
              >
                {c}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="divide-y divide-line">
          {rows.map((row) => (
            <tr key={rowKey(row)} className="transition-colors hover:bg-hover">
              {children(row)}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Shared cell padding so callers only supply content. */
export const td = "px-3 py-1.5 align-middle text-muted";
export const tdStrong = "px-3 py-1.5 align-middle font-medium text-fg";

/** A `<kbd>` in the keyboard-hint rows. Six of these appeared verbatim across
 *  the composer's hint bar and the transcript's welcome screen. */
export function Kbd({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <kbd
      className={cn(
        "rounded-[5px] border border-line bg-surface px-1.5 py-px font-mono text-2xs leading-4 text-muted",
        className,
      )}
    >
      {children}
    </kbd>
  );
}

/** Nav-style row: icon + label, with a selected state.
 *
 *  This exact utility string was pasted into the sidebar's nav, the sidebar's
 *  project list and the settings nav. The focus ring is not restated — the
 *  global `:focus-visible` rule in tokens.css covers it. */
export function NavRow({
  icon,
  children,
  selected = false,
  trailing,
  className,
  ...props
}: {
  icon?: ReactNode;
  children: ReactNode;
  selected?: boolean;
  trailing?: ReactNode;
  className?: string;
} & Omit<React.ComponentProps<"button">, "className" | "children">) {
  return (
    <button
      type="button"
      aria-current={selected ? "page" : undefined}
      className={cn(
        "flex h-row w-full items-center gap-2 rounded-lg px-2.5 text-sm",
        "text-muted transition-colors duration-100 hover:bg-hover hover:text-fg",
        selected && "bg-brand-soft font-medium text-fg",
        className,
      )}
      {...props}
    >
      {icon && <span className="shrink-0 text-faint">{icon}</span>}
      <span className="min-w-0 flex-1 truncate text-start">{children}</span>
      {trailing}
    </button>
  );
}

/** Group heading above a sidebar/settings nav section. */
export function SectionLabel({
  children,
  trailing,
  className,
}: {
  children: ReactNode;
  trailing?: ReactNode;
  className?: string;
}) {
  return (
    <h2
      className={cn(
        "flex items-center gap-1 px-2.5 pb-1 text-2xs font-semibold uppercase tracking-[0.08em] text-faint",
        className,
      )}
    >
      <span className="truncate">{children}</span>
      {trailing}
    </h2>
  );
}

/** A selectable row inside a list, sized by the shared `--row` height. */
export function ListRow({
  children,
  selected = false,
  className,
  ...props
}: {
  children: ReactNode;
  selected?: boolean;
  className?: string;
} & Omit<React.ComponentProps<"button">, "className" | "children">) {
  return (
    <button
      type="button"
      aria-current={selected ? "true" : undefined}
      className={cn(
        "flex h-row w-full items-center gap-2 rounded-lg px-2.5 text-start",
        "text-muted transition-colors duration-100 hover:bg-hover hover:text-fg",
        selected && "bg-brand-soft font-medium text-fg",
        className,
      )}
      {...props}
    >
      {children}
    </button>
  );
}

/** Header row of a side panel: icon, title, optional trailing controls, close.
 *
 * Every right-hand surface retyped this — the height, the border, the back
 * arrow, and the close button together with its translated label and
 * `aria-label`. Four copies of the same button is four chances for one of them
 * to lose its accessible name.
 */
export function PanelHeader({
  icon,
  title,
  onClose,
  onBack,
  trailing,
  titleClass = "font-mono text-xs",
  className,
}: {
  icon: ReactNode;
  title: string;
  onClose: () => void;
  onBack?: () => void;
  /** Controls between the title and the close button (badges, refresh, …). */
  trailing?: ReactNode;
  titleClass?: string;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex h-titlebar shrink-0 items-center gap-1.5 border-b border-line bg-surface/80 px-3 backdrop-blur",
        className,
      )}
    >
      {onBack ? (
        <Button variant="ghost" size="icon-sm" title={t("戻る")} aria-label={t("戻る")} onClick={onBack}>
          <ArrowLeft size={13} />
        </Button>
      ) : (
        <span className="shrink-0 text-faint">{icon}</span>
      )}
      <span className={cn("min-w-0 flex-1 truncate text-fg", titleClass)} title={title}>
        {title}
      </span>
      {trailing}
      <Button variant="ghost" size="icon-sm" title={t("閉じる")} aria-label={t("閉じる")} onClick={onClose}>
        <X size={13} />
      </Button>
    </div>
  );
}

