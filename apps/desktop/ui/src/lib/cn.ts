import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Conditional class names with Tailwind conflict resolution. `twMerge` makes
 * the last utility win, so a caller can override a component's defaults by
 * simply repeating the property: `cn("p-2", "p-4")` → `"p-4"`.
 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
