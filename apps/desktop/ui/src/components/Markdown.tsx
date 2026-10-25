import { memo, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkBreaks from "remark-breaks";
import { ExternalLink } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { CodeBlock } from "./CopyButton";

/** Open http(s) links in the system browser (validated Rust-side). */
function isExternalHref(href: string): boolean {
  return href.startsWith("http://") || href.startsWith("https://");
}

function onLinkClick(event: React.MouseEvent<HTMLAnchorElement>, href: string) {
  // Neutralize javascript:/data:/vbscript: and anything else non-http(s).
  // Always preventDefault so a hostile href can never navigate the WebView.
  event.preventDefault();
  if (!isExternalHref(href)) return;
  void api.openExternal(href);
}

// Hoisted so the props are referentially stable across renders. Rebuilding
// them here handed react-markdown a fresh plugin array and component map on
// every render, which made the `memo` below unable to bail out — so a live
// streaming message re-parsed the whole accumulated markdown ~16×/s.
const REMARK_PLUGINS = [remarkGfm, remarkBreaks];
const COMPONENTS = {
  a({ href, children }: { href?: string; children?: ReactNode }) {
    const external = !!href && /^https?:\/\//.test(href);
    return (
      <a
        href={external ? href : undefined}
        target={external ? "_blank" : undefined}
        rel="noopener noreferrer"
        onClick={(e) => href && onLinkClick(e, href)}
        className={external ? "inline-flex items-center gap-0.5" : undefined}
      >
        {children}
        {external && (
          <ExternalLink
            size={12}
            aria-hidden="true"
            className="shrink-0 text-faint"
          />
        )}
      </a>
    );
  },
  pre({ children }: { children?: ReactNode }) {
    // react-markdown hands <pre><code class="language-x">…</code></pre>
    // Extract the raw text + language so CodeBlock can render a header.
    const child = Array.isArray(children) ? children[0] : children;
    if (child && typeof child === "object" && "props" in child) {
      const props = (
        child as { props?: { className?: string; children?: ReactNode } }
      ).props;
      const language = /language-(\w+)/.exec(props?.className ?? "")?.[1] ?? "";
      const code = toText(props?.children);
      return <CodeBlock language={language} code={code} />;
    }
    return <pre>{children}</pre>;
  },
  table({ children }: { children?: ReactNode }) {
    // Wide GFM tables overflow a narrow transcript column. The scroll region
    // is keyboard-focusable with its own accessible name so keyboard and
    // screen-reader users can reach columns that overflow horizontally.
    return (
      // A scrollable `region` with a label is the documented pattern for
      // keyboard-reachable overflow (same exception as `ResizableHandle`).
      <div
        className="overflow-x-auto"
        // eslint-disable-next-line jsx-a11y/no-noninteractive-tabindex
        tabIndex={0}
        role="region"
        aria-label={t("表")}
      >
        <table>{children}</table>
      </div>
    );
  },
};

/**
 * Markdown renderer for assistant messages.
 *
 * - GFM tables/strikethrough + single-newline line breaks (chat convention)
 * - fenced code blocks get a language header and a copy button
 * - raw HTML is not enabled: react-markdown escapes it by default
 */
export const Markdown = memo(function Markdown({
  children,
}: {
  children: string;
}) {
  return (
    <div className="markdown-body">
      <ReactMarkdown remarkPlugins={REMARK_PLUGINS} components={COMPONENTS}>
        {children}
      </ReactMarkdown>
    </div>
  );
});

function toText(node: ReactNode): string {
  if (node == null || typeof node === "boolean") return "";
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(toText).join("");
  if (typeof node === "object" && "props" in node) {
    return toText(
      (node as { props?: { children?: ReactNode } }).props?.children,
    );
  }
  return "";
}
