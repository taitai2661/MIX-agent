import { getLanguage } from "@/lib/i18n";
import { greetingEn } from "./translations";

/**
 * Translate a greeting string by its stable id.
 *
 * The Japanese text is canonical and doubles as the fallback: when the UI is
 * Japanese, or an id is missing from the map, the original is returned as-is.
 * Keying by id (not by the Japanese text) means editing the wording in
 * `pools.ts` / `suggestions.ts` cannot silently drop the English translation.
 */
export function greetingText(id: string, fallback: string): string {
  if (getLanguage() === "ja") return fallback;
  return greetingEn[id] ?? fallback;
}

/** A suggestion carries two translated strings; their keys are namespaced. */
export function suggestionLabelId(id: string): string {
  return `${id}.label`;
}

export function suggestionPromptId(id: string): string {
  return `${id}.prompt`;
}
