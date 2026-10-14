import type { ReactNode } from "react";
import { Search, Trash2 } from "lucide-react";
import { t } from "@/lib/i18n";
import { cn } from "@/lib/cn";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { useConfirm } from "./ui/dialog";
import { Input } from "./ui/input";
import { Spinner } from "./ui/spinner";
import { Switch } from "./ui/switch";
import { EmptyState, ErrorBox, ListSkeleton, TableSkeleton } from "./shared";

/**
 * Shared pane primitives + list-state wrapper.
 *
 * Several panes retyped the same blocks (search row, delete icon button,
 * toggle row, section heading, pending/error/empty ladder). They now live
 * here so spacing, tones and accessible names stay identical everywhere.
 */

/** Single-line search row: expanding input + primary search button. */
export function SearchForm({
  value,
  onChange,
  onSubmit,
  searching = false,
  placeholder,
  searchLabel,
  className,
}: {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  searching?: boolean;
  placeholder?: string;
  searchLabel?: string;
  className?: string;
}) {
  const label = searchLabel ?? t("検索");
  return (
    <div className={cn("flex gap-1.5", className)}>
      <Input
        className="min-w-0 flex-1"
        placeholder={placeholder ?? t("キーワード")}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && value.trim() && !searching) {
            e.preventDefault();
            onSubmit();
          }
        }}
      />
      <Button disabled={!value.trim() || searching} onClick={onSubmit}>
        {searching ? <Spinner size={12} /> : <Search size={12} />} {label}
      </Button>
    </div>
  );
}

/**
 * Self-contained destructive icon button.
 *
 * Nine panes repeated `Button ghost-danger + Trash2 + await confirm(...) +
 * mutate`. The dialog lives inside, so call sites drop from ~15 lines to 5
 * and cannot forget to render `{dialog}` (which previously left the button
 * doing nothing).
 */
export function DeleteIconButton({
  title,
  description,
  confirmLabel,
  onDelete,
  pending = false,
  disabled = false,
  label,
}: {
  title: string;
  description?: string;
  confirmLabel?: string;
  onDelete: () => void | Promise<void>;
  pending?: boolean;
  disabled?: boolean;
  /** Accessible name; defaults to the translated "削除". */
  label?: string;
}) {
  const { confirm, dialog } = useConfirm();
  const name = label ?? t("削除");
  return (
    <>
      <Button
        variant="ghost-danger"
        size="icon-sm"
        title={name}
        aria-label={name}
        disabled={disabled || pending}
        onClick={async () => {
          if (
            await confirm({
              title,
              description,
              confirmLabel: confirmLabel ?? t("削除"),
              tone: "danger",
            })
          ) {
            await onDelete();
          }
        }}
      >
        {pending ? <Spinner size={12} /> : <Trash2 size={13} />}
      </Button>
      {dialog}
    </>
  );
}

/** Label + switch on one row; replaces the duplicated local `Toggle`. */
export function ToggleRow({
  label,
  hint,
  checked,
  onChange,
  disabled = false,
  className,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <div className={cn("flex flex-col gap-1", className)}>
      <label className="flex items-center justify-between gap-4 text-sm text-fg">
        <span>{label}</span>
        <Switch checked={checked} onCheckedChange={onChange} disabled={disabled} />
      </label>
      {hint && <p className="m-0 text-xs text-muted">{hint}</p>}
    </div>
  );
}

/** Section heading inside a pane body; one size everywhere. */
export function SectionHeading({ children, className }: { children: ReactNode; className?: string }) {
  return <h2 className={cn("text-sm font-semibold text-fg", className)}>{children}</h2>;
}

/** Compact count chip for nav labels, panel headers and table titles. */
export function CountBadge({ count, label }: { count: number; label?: string }) {
  return (
    <Badge tone="neutral" size="sm" className="font-mono" title={label}>
      {count}
    </Badge>
  );
}

type QueryLike<T> = {
  isPending: boolean;
  isError: boolean;
  error: unknown;
  data: T | undefined;
};

/**
 * Standard pane read-state ladder: skeleton → error → empty → content.
 *
 * Six panes pasted the same `isPending ? Skeleton : isError ? ErrorBox :
 * empty ? EmptyState : content` chain with only the icon/title differing.
 * Centralising it removes the branch drift (one pane showed a table skeleton,
 * another a card skeleton, for the same table) and guarantees a failed fetch
 * never renders as "no items".
 */
export function PaneQueryState<T>({
  query,
  isEmpty,
  empty,
  skeleton,
  children,
  className,
}: {
  query: QueryLike<T>;
  isEmpty: (data: T) => boolean;
  empty: ReactNode;
  skeleton?: ReactNode;
  children: (data: T) => ReactNode;
  className?: string;
}) {
  if (query.isPending) {
    return <>{skeleton ?? <TableSkeleton />}</>;
  }
  if (query.isError) {
    return <ErrorBox error={query.error} className={className} />;
  }
  // A successful fetch with no data (e.g. a disabled query resolving
  // `undefined`) is not a failure: rendering `ErrorBox(error={null})` here
  // showed a misleading "unknown error" instead of the empty state.
  if (query.data == null) {
    return <>{empty}</>;
  }
  if (isEmpty(query.data)) {
    return <>{empty}</>;
  }
  return <>{children(query.data)}</>;
}

/** Empty state for a filtered list with a one-tap filter reset. */
export function SearchEmptyState({
  title,
  sub,
  onClear,
  clearLabel,
  compact = false,
}: {
  title: string;
  sub?: string;
  onClear: () => void;
  clearLabel?: string;
  /** Dense variant for sidebars and side panels (see `EmptyState`). */
  compact?: boolean;
}) {
  return (
    <EmptyState
      title={title}
      sub={sub}
      compact={compact}
      action={
        <Button size="sm" onClick={onClear}>
          {clearLabel ?? t("絞り込みをクリア")}
        </Button>
      }
    />
  );
}

/** Skeleton for a popover listbox (model picker, command palette). */
export function ListboxSkeleton({ rows = 3 }: { rows?: number }) {
  return (
    <div className="flex flex-col gap-1.5 p-1.5" aria-hidden="true">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="h-9 animate-pulse rounded-md bg-inset" />
      ))}
    </div>
  );
}

/** Re-export so existing `shared.tsx` call sites can migrate gradually. */
export { EmptyState, ErrorBox, ListSkeleton, TableSkeleton };
