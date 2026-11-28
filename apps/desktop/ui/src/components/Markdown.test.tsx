import { fireEvent, render, screen } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/api";
import { setLanguage } from "@/lib/i18n";
import { isExternalHref, Markdown } from "./Markdown";

beforeAll(() => setLanguage("ja"));

vi.mock("@/lib/api", () => ({
  api: {
    openExternal: vi.fn(async () => undefined),
  },
}));

const mockedOpenExternal = vi.mocked(api.openExternal);

beforeEach(() => vi.clearAllMocks());

describe("desktop Markdown link guard", () => {
  it.each([
    ["javascript", "javascript:alert(1)"],
    ["data", "data:text/html,<script>alert(1)</script>"],
    ["vbscript", "vbscript:msgbox(1)"],
    ["file", "file:///etc/passwd"],
    ["relative", "/local/path"],
    ["mailto", "mailto:user@example.com"],
    ["control char in scheme", "java\tscript:alert(1)"],
  ])("never sends a %s href to openExternal", (_label, href) => {
    expect(isExternalHref(href)).toBe(false);
    const { container } = render(<Markdown>{`[クリック](${href})`}</Markdown>);
    const link = container.querySelector("a");
    if (link) fireEvent.click(link);
    expect(mockedOpenExternal).not.toHaveBeenCalled();
  });

  it("opens a safe http(s) link in the system browser", () => {
    render(<Markdown>{"[クリック](https://example.com/ok)"}</Markdown>);
    const link = screen.getByRole("link", { name: /クリック/ });
    expect(link.getAttribute("href")).toBe("https://example.com/ok");
    fireEvent.click(link);
    expect(mockedOpenExternal).toHaveBeenCalledWith("https://example.com/ok");
  });

  it("rejects a control-character host even when the scheme looks safe", () => {
    expect(isExternalHref("https://exa\tmple.com")).toBe(false);
  });

  it("opens the system browser on middle-click too", () => {
    render(<Markdown>{"[クリック](https://example.com/ok)"}</Markdown>);
    const link = screen.getByRole("link", { name: /クリック/ });
    fireEvent(
      link,
      new MouseEvent("auxclick", {
        bubbles: true,
        cancelable: true,
        button: 1,
      }),
    );
    expect(mockedOpenExternal).toHaveBeenCalledWith("https://example.com/ok");
  });
});
