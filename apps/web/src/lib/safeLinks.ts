/**
 * Shared URL guards for model- and tool-supplied links.
 *
 * Browsers ignore control characters while parsing a URL scheme, so a string
 * such as `java\tscript:alert(1)` can pass a naive `startsWith("javascript:")`
 * check and still execute when used as an `href`. Reject raw control bytes
 * before inspecting the scheme, and keep the allow-list explicit.
 */

function hasControlChars(value: string): boolean {
  for (const char of value) {
    const code = char.codePointAt(0) ?? 0;
    if (code <= 0x1f || code === 0x7f) return true;
  }
  return false;
}

const HTTP_SCHEME = /^https?:\/\//i;
const ANY_SCHEME = /^([a-z][a-z0-9+.-]*):/i;
/** Schemes safe in an inline markdown link. Relative/anchor links have no scheme. */
const SAFE_INLINE_SCHEMES = new Set(["http", "https", "mailto"]);

/** External links rendered by tool activity / result panels: http(s) only. */
export function isSafeHttpUrl(value: unknown): value is string {
  if (typeof value !== "string") return false;
  const url = value.trim();
  if (!url || hasControlChars(value)) return false;
  return HTTP_SCHEME.test(url);
}

/** Inline markdown links: relative URLs, mailto and http(s) are all fine. */
export function isSafeInlineHref(value: unknown): value is string {
  if (typeof value !== "string") return false;
  const href = value.trim();
  if (!href || hasControlChars(value)) return false;
  // `//host` is a network-path reference (external), and browsers normalise a
  // leading backslash to `/` for special URLs, so `\\host` is external too.
  // Neither is a same-document relative link despite having no scheme.
  if (href.startsWith("//") || href.startsWith("\\")) return false;
  const scheme = ANY_SCHEME.exec(href);
  return scheme === null || SAFE_INLINE_SCHEMES.has(scheme[1].toLowerCase());
}
