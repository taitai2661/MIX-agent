/// Display-language switcher, mirroring the Docker edition's approach
/// (`docker/MIX-agent/apps/web/src/app/i18n.ts`): Japanese strings are the
/// canonical keys and `t()` maps them to English when the UI language is `en`.
///
/// New code that needs stable identifiers (rather than copy that translators
/// may reword) should use id keys like the chat greeting does
/// (`features/chat/greeting/translations.ts`, `greetingText(id, fallback)`):
/// keying by id means editing the Japanese wording cannot silently drop the
/// English translation.

import { getString, setString } from "@/lib/storage";
import { en } from "./i18n/en";

export type Language = "ja" | "en";

export { en };

const STORAGE_KEY = "mix-agent-language";

export function browserLanguage(
  languages: readonly string[] = typeof navigator === "undefined"
    ? []
    : navigator.languages,
): Language {
  for (const value of languages) {
    if (value.toLowerCase().startsWith("ja")) return "ja";
    if (value.toLowerCase().startsWith("en")) return "en";
  }
  return "ja";
}

let language: Language = initialLanguage();

function initialLanguage(): Language {
  const stored = getString(STORAGE_KEY);
  if (stored === "ja" || stored === "en") return stored;
  return browserLanguage();
}

export function getLanguage(): Language {
  return language;
}

export function setLanguage(value: Language): void {
  if (language === value) return;
  language = value;
  setString(STORAGE_KEY, value);
  if (typeof document !== "undefined") document.documentElement.lang = value;
}

if (typeof document !== "undefined") document.documentElement.lang = language;

/** The Japanese-key dictionary itself, exported for tests and tooling.
 *
 *  The empty-chat greeting is no longer keyed here: it lives in
 *  `features/chat/greeting/translations.ts`, keyed by stable fragment id, so
 *  its Japanese wording can change without breaking the English text. */
export const dictionary = en;

/** Translate a Japanese UI key into the active display language.
 *
 *  In `ja` mode the key itself is the copy, so this is the identity function;
 *  in `en` mode it consults the table in `./i18n/en`. Legacy English-literal
 *  keys (`t("Copy")`, …) were rewritten to their Japanese-canonical forms, so
 *  no reverse map is needed anymore. */
export function t(value: string): string {
  if (language === "ja") return value;
  return en[value] ?? value;
}

/** `t` with `{placeholder}` interpolation. */
export function tf(
  value: string,
  params: Record<string, string | number>,
): string {
  let out = t(value);
  for (const [key, replacement] of Object.entries(params)) {
    out = out.split(`{${key}}`).join(String(replacement));
  }
  return out;
}
