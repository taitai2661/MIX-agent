import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { isSafeInlineHref } from "@/lib/safeLinks";
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

  // Model output is untrusted: only http(s), mailto and relative links may
  // become anchors. Scheme-looking strings with embedded control characters
  // are rejected too (browsers ignore those bytes when resolving a URL).
  it.each([
    ["javascript", "javascript:alert(1)"],
    ["data", "data:text/html;base64,PHNjcmlwdD4="],
    ["vbscript", "vbscript:msgbox(1)"],
    ["file", "file:///etc/passwd"],
    ["control char in scheme", "java\tscript:alert(1)"],
    ["network-path reference", "//evil.example/phish"],
  ])("renders a %s link as inert text", (_label, url) => {
    const html = renderToStaticMarkup(
      <ChatMarkdown>{`[クリック](<${url}>)`}</ChatMarkdown>,
    );
    expect(html).toContain("クリック");
    expect(html).not.toContain("href=");
  });

  // react-markdown percent-encodes a leading backslash before the link
  // component sees it, so assert the guard itself for that raw input.
  it("rejects raw backslash network-path references at the guard", () => {
    expect(isSafeInlineHref("\\\\evil.example/phish")).toBe(false);
    expect(isSafeInlineHref("\\evil.example/phish")).toBe(false);
  });

  it("never puts a raw control character into an href", () => {
    const html = renderToStaticMarkup(
      <ChatMarkdown>{`[クリック](<https://exa\tmple.com>)`}</ChatMarkdown>,
    );
    // The markdown parser percent-encodes the tab; the guard is defence in
    // depth for any caller that hands us a raw URL from another source.
    const href = /href="([^"]*)"/.exec(html)?.[1] ?? "";
    expect(Array.from(href).some((c) => (c.codePointAt(0) ?? 0) <= 0x1f)).toBe(
      false,
    );
  });

  it.each([
    ["mailto", "mailto:user@example.com"],
    ["relative", "/docs/intro"],
    ["anchor", "#section"],
    ["https", "https://example.com/ok"],
  ])("keeps a %s link clickable", (_label, url) => {
    const html = renderToStaticMarkup(
      <ChatMarkdown>{`[クリック](${url})`}</ChatMarkdown>,
    );
    expect(html).toContain(`href="${url}"`);
  });
});
