import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { ChatMarkdown } from "./ChatMarkdown";

describe("ChatMarkdown", () => {
  it("renders GFM and opens links in a new tab", () => {
    const html = renderToStaticMarkup(
      <ChatMarkdown>
        {"- 項目\n\n[リンク](https://example.com)\n\n次の行"}
      </ChatMarkdown>,
    );
    expect(html).toContain("<li>項目</li>");
    expect(html).toContain('href="https://example.com"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noopener noreferrer"');
    expect(html).toContain("次の行");
  });
});
