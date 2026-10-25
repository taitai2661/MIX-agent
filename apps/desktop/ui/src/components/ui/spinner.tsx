import { Loader2 } from "lucide-react";
import { cn } from "@/lib/cn";

/**
 * The app's only spinner.
 *
 * It replaces two mechanisms that ran at different speeds: Tailwind's
 * `animate-spin` (1s) and a local `.spin` keyframe (900ms), both of which
 * appeared in the same view. The icon is *replaced* rather than overlaid so
 * that callers no longer have to hand-roll a conditional — a `<RefreshCw>` that
 * spins when a mutation is pending was open-coded in eight places.
 */
export function Spinner({
  size = 13,
  className,
  label,
}: {
  size?: number;
  className?: string;
  /** Announced to screen readers. Defaults to nothing (`aria-hidden`): callers
   *  that need it should already have visible text nearby, e.g. "読み込み中…". */
  label?: string;
}) {
  return (
    <Loader2
      size={size}
      className={cn("shrink-0 animate-spin text-faint", className)}
      aria-hidden={label ? undefined : true}
      aria-label={label}
      role={label ? "status" : undefined}
    />
  );
}
