import { memo, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkBreaks from "remark-breaks";
import { api } from "@/lib/api";
import { CodeBlock } from "./CopyButton";

/** Open http(s) links in the system browser (validated Rust-side). */
function onLinkClick(event: React.MouseEvent<HTMLAnchorElement>, href: string) {
  if (!href.startsWith("http://") && !href.startsWith("https://")) return;
  event.preventDefault();
  void api.openExternal(href);
}

// Hoisted so the props are referentially stable across renders. Rebuilding
// them here handed react-markdown a fresh plugin array and component map on
// every render, which made the `memo` below unable to bail out — so a live
// streaming message re-parsed the whole accumulated markdown ~16×/s.
const REMARK_PLUGINS = [remarkGfm, remarkBreaks];
const COMPONENTS = {
  a({ href, children }: { href?: string; children?: ReactNode }) {
    return (
      <a
        href={href}
        target="_blank"
        rel="noreferrer"
        onClick={(e) => href && onLinkClick(e, href)}
      >
        {children}
      </a>
    );
  },
  pre({ children }: { children?: ReactNode }) {
    // react-markdown hands <pre><code class="language-x">…</code></pre>
    // Extract the raw text + language so CodeBlock can render a header.
    const child = Array.isArray(children) ? children[0] : children;
    if (child && typeof child === "object" && "props" in child) {
      const props = (child as { props?: { className?: string; children?: ReactNode } }).props;
      const language = /language-(\w+)/.exec(props?.className ?? "")?.[1] ?? "";
      const code = toText(props?.children);
      return <CodeBlock language={language} code={code} />;
    }
    return <pre>{children}</pre>;
  },
};

/**
 * Markdown renderer for assistant messages.
 *
 * - GFM tables/strikethrough + single-newline line breaks (chat convention)
 * - fenced code blocks get a language header and a copy button
 * - raw HTML is not enabled: react-markdown escapes it by default
 */
export const Markdown = memo(function Markdown({ children }: { children: string }) {
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
    return toText((node as { props?: { children?: ReactNode } }).props?.children);
  }
  return "";
}
