import Markdown from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

/** Shared markdown renderer: GFM + soft line breaks, links open in a new tab.
 * Relative links and anchors stay usable; only URLs with a dangerous scheme
 * (javascript:, data:, vbscript:, file:, ...) fall back to a plain span so
 * model output cannot inject an executable href. */
const SAFE_SCHEMES = new Set(["http", "https", "mailto"]);

function isSafeHref(href: unknown): href is string {
  if (typeof href !== "string") return false;
  const value = href.trim();
  if (!value) return false;
  // Browsers ignore control characters when parsing a scheme, so strip them
  // before deciding whether this is relative or uses a dangerous protocol.
  const compact = Array.from(value)
    .filter((char) => {
      const code = char.codePointAt(0) ?? 0;
      return code > 0x20 && code !== 0x7f;
    })
    .join("");
  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(compact);
  return scheme === null || SAFE_SCHEMES.has(scheme[1].toLowerCase());
}

export function ChatMarkdown({ children }: { children: string }) {
  return (
    <Markdown
      remarkPlugins={[remarkGfm, remarkBreaks]}
      components={{
        a: ({ node: _node, children: linkChildren, href, ...props }) =>
          isSafeHref(href) ? (
            <a {...props} href={href} target="_blank" rel="noopener noreferrer">
              {linkChildren}
            </a>
          ) : (
            <span>{linkChildren}</span>
          ),
      }}
    >
      {children}
    </Markdown>
  );
}
