import { t } from "./i18n";
import type { BadgeProps } from "@/components/ui/badge";

/**
 * Git's porcelain status codes, mapped once.
 *
 * The file tree and the changes panel each carried their own copy of this
 * mapping, and the two had drifted: the changes panel recognised `R` and `C`
 * while the tree folded them into "modified". A row's meaning therefore depended
 * on which panel it was rendered in.
 */
export function gitStatusLabel(status: string): string {
  if (status.includes("?")) return t("未追跡");
  if (status.includes("D")) return t("削除");
  if (status.includes("A")) return t("追加");
  if (status.includes("R")) return t("改名");
  if (status.includes("C")) return t("コピー");
  return t("変更");
}

export function gitStatusTone(status: string): NonNullable<BadgeProps["tone"]> {
  if (status.includes("?")) return "info";
  if (status.includes("D")) return "danger";
  if (status.includes("A")) return "success";
  return "warning";
}

export function gitStatusLetter(status: string): string {
  if (status.includes("?")) return "?";
  return status.trim().slice(-1) || "M";
}
