import Markdown from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

/** Shared markdown renderer: GFM + soft line breaks, links open in a new tab. */
export function ChatMarkdown({ children }: { children: string }) {
  return (
    <Markdown
      remarkPlugins={[remarkGfm, remarkBreaks]}
      components={{
        a: ({ node: _node, children: linkChildren, ...props }) => (
          <a {...props} target="_blank" rel="noopener noreferrer">
            {linkChildren}
          </a>
        ),
      }}
    >
      {children}
    </Markdown>
  );
}
