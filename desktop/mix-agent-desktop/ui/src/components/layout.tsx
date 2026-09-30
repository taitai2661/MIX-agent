import type { ReactNode } from "react";
import { ArrowLeft, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { Button } from "./ui/button";
import { Label } from "./ui/label";

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
  return (
    <div className={cn("flex min-h-0 flex-1 flex-col overflow-y-auto", className)}>
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 px-6 py-6">
        {(title || actions) && (
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              {title && (
                <h2 className="text-lg font-semibold tracking-tight text-fg">{title}</h2>
              )}
              {sub && <p className="mt-0.5 text-sm text-muted">{sub}</p>}
            </div>
            {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
          </div>
        )}
        {children}
      </div>
    </div>
  );
}

/** Vertical stack of cards, the standard body of a settings tab. */
export function Section({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("flex flex-col gap-4", className)}>{children}</div>;
}

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
        "flex flex-col overflow-hidden rounded-lg border border-line bg-surface shadow-xs",
        className,
      )}
    >
      {(title || action) && (
        <div className="flex items-center gap-1.5 border-b border-line px-3 py-2">
          {icon && <span className="shrink-0 text-faint">{icon}</span>}
          {title && <h3 className="min-w-0 flex-1 text-sm font-semibold text-fg">{title}</h3>}
          {action}
        </div>
      )}
      <div className={cn("flex flex-col gap-2 p-3", bodyClassName)}>{children}</div>
    </section>
  );
}

/** Label + control + optional hint, with consistent vertical rhythm. */
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
  return (
    <label className={cn("flex flex-col gap-1.5", className)}>
      {label && <Label>{label}</Label>}
      {children}
      {hint && <span className="text-2xs text-faint">{hint}</span>}
    </label>
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
    <div className="overflow-x-auto rounded-lg border border-line">
      <table className="w-full border-collapse text-xs">
        <thead>
          <tr className="border-b border-line bg-inset text-2xs text-muted">
            {columns.map((c, i) => (
              <th key={i} className="px-3 py-1.5 text-start font-medium whitespace-nowrap">
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
        "flex h-9 shrink-0 items-center gap-1.5 border-b border-line px-3",
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

