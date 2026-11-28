import { useRef, type ComponentProps, type ReactNode } from "react";
import Markdown from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

import { t } from "@/app/i18n";
import { CopyButton } from "@/components/copy-button";
import { isSafeInlineHref } from "@/lib/safeLinks";

/** Fenced code blocks get their own copy affordance; the whole message also
 * has a copy action, but copying a single snippet is the common case. */
function CodeBlock({ children }: { children?: ReactNode }) {
  const ref = useRef<HTMLPreElement>(null);
  return (
    <div className="code-block">
      <CopyButton
        value={() => ref.current?.innerText ?? ""}
        label={t("コードをコピー")}
        className="code-copy"
      />
      <pre ref={ref}>{children}</pre>
    </div>
  );
}

// Hoisted: react-markdown re-parses whenever these identities change, and a
// live stream re-renders many times per second. Stable references let memoized
// parents bail out while only the message whose text changed is re-parsed.
const REMARK_PLUGINS = [remarkGfm, remarkBreaks];
type MarkdownLinkProps = ComponentProps<"a"> & { node?: unknown };
const COMPONENTS = {
  a: ({
    node: _node,
    children: linkChildren,
    href,
    ...props
  }: MarkdownLinkProps) =>
    isSafeInlineHref(href) ? (
      <a {...props} href={href} target="_blank" rel="noopener noreferrer">
        {linkChildren}
      </a>
    ) : (
      <span>{linkChildren}</span>
    ),
  pre: ({ children: codeChildren }: { children?: ReactNode }) => (
    <CodeBlock>{codeChildren}</CodeBlock>
  ),
};

/** Shared markdown renderer: GFM + soft line breaks, links open in a new tab.
 * Relative links and mailto stay usable; only URLs with a dangerous scheme
 * (javascript:, data:, vbscript:, file:, …) fall back to a plain span. */
export function ChatMarkdown({ children }: { children: string }) {
  return (
    <Markdown remarkPlugins={REMARK_PLUGINS} components={COMPONENTS}>
      {children}
    </Markdown>
  );
}
