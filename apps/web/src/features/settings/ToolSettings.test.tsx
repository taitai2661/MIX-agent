import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { BrowserSettings } from "./ToolSettings";

it("shows the overall Browser install percentage and stage", () => {
  const client = new QueryClient();
  client.setQueryData(["/settings"], {
    data: {
      browser_install_status: "installing",
      browser_install_progress: 42,
      browser_install_stage: "ダウンロード中",
    },
  });
  const html = renderToStaticMarkup(
    <QueryClientProvider client={client}><MemoryRouter><BrowserSettings /></MemoryRouter></QueryClientProvider>,
  );
  expect(html).toContain("インストール中… 42%");
  expect(html).toContain("ダウンロード中");
});
