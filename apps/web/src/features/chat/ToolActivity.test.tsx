import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";

import { ToolActivity } from "./ToolActivity";

describe("ToolActivity", () => {
  it("shows the live search phrase without source links", () => {
    const html = renderToStaticMarkup(
      <ToolActivity
        running
        activity={{
          icon: "search",
          label: "Webを検索中",
          detail: "AIニュース",
        }}
      />,
    );
    expect(html).toContain("AIニュース を検索中");
    expect(html).not.toContain("href=");
  });

  it("keeps completed search sources as safe external links", () => {
    const html = renderToStaticMarkup(
      <ToolActivity
        activity={{
          icon: "search",
          label: "2件のWebサイトを検索しました",
          sources: [{ host: "example.com", url: "https://example.com/news" }],
          remaining: 1,
        }}
      />,
    );
    expect(html).toContain("2件のWebサイトを検索しました");
    expect(html).toContain('href="https://example.com/news"');
    expect(html).toContain("あと 1 件");
  });

  // A hostile tool result must never turn a source label into a clickable
  // javascript:/data: URL (or one hidden behind an ignored control byte).
  it.each([
    ["javascript", "javascript:alert(1)"],
    ["data", "data:text/html,<script>alert(1)</script>"],
    ["control char in scheme", "java\tscript:alert(1)"],
    ["control char in host", "https://exa\nmple.com"],
    ["mailto", "mailto:someone@example.com"],
    ["relative", "/local/path"],
  ])("renders %s sources as inert text", (_label, url) => {
    const html = renderToStaticMarkup(
      <ToolActivity
        activity={{
          label: "検索結果",
          sources: [{ host: "example.com", url }],
        }}
      />,
    );
    expect(html).toContain("example.com");
    expect(html).not.toContain("href=");
  });
});
