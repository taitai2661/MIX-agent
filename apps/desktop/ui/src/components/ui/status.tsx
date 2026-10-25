import { Badge, type BadgeProps } from "./badge";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";

/**
 * On/off state as a badge.
 *
 * Every "enabled / vision / tools" cell in the app rendered a bare `✓` with no
 * text, so the state was carried by colour alone and a screen reader announced
 * nothing at all. The tick is now decorative and the word is real, with the
 * badge's accessible name coming from the visible label.
 */
export function YesNo({
  value,
  onLabel = t("対応"),
  offLabel = "—",
  trueTone = "success",
}: {
  value: boolean;
  onLabel?: string;
  offLabel?: string;
  trueTone?: BadgeProps["tone"];
}) {
  return (
    <Badge tone={value ? trueTone : "neutral"} size="sm">
      {value ? onLabel : offLabel}
    </Badge>
  );
}

/**
 * A single dot plus a hidden word.
 *
 * Same reasoning as [`YesNo`]: the app's health, connection and lifecycle
 * indicators were colour-only.
 */
export function StatusDot({
  tone,
  label,
  className,
}: {
  tone: "success" | "warning" | "danger" | "info" | "neutral";
  /** Visible to a screen reader, hidden visually. */
  label: string;
  className?: string;
}) {
  const fill = {
    success: "bg-success",
    warning: "bg-warning",
    danger: "bg-danger",
    info: "bg-info",
    neutral: "bg-line-strong",
  }[tone];
  return (
    <span className={cn("inline-flex items-center gap-1.5", className)}>
      <span
        aria-hidden="true"
        className={cn("size-2 shrink-0 rounded-full", fill)}
      />
      <span className="sr-only">{label}</span>
    </span>
  );
}

/** Git's two-letter porcelain status, as a badge with a readable name. */
export function GitStatusBadge({
  letter,
  label,
  tone,
}: {
  letter: string;
  label: string;
  tone: BadgeProps["tone"];
}) {
  return (
    <Badge tone={tone} size="sm" className="shrink-0 font-mono">
      <span aria-hidden="true">{letter}</span>
      <span className="sr-only">{label}</span>
    </Badge>
  );
}
