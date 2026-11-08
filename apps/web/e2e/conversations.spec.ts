import { expect, test } from "@playwright/test";

import { CONVERSATION_PAGE_SIZE } from "../src/lib/conversations";

/**
 * The sidebar asks for one bounded page at a time. A long-lived installation
 * has more than one page of active threads, and before this flow existed the
 * list silently stopped at the server default with no way to reach older
 * conversations from the sidebar.
 */
test("sidebar pages through older conversations", async ({ page }) => {
  // The first-run tour is a modal over the shell; dismiss it before clicking.
  await page.addInitScript(() => {
    localStorage.setItem("mix-agent-tour-done", "1");
  });

  const row = (index: number) => ({
    id: `c${index}`,
    data: {
      title: `会話 ${index}`,
      pinned: false,
      last_message_at: "2026-01-01T00:00:00Z",
    },
    created_at: "2026-01-01T00:00:00Z",
  });

  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.replace(/^\/api\/v1/, "");

    if (path === "/setup") {
      return route.fulfill({ json: { needs_admin: false } });
    }
    if (path === "/auth/me") {
      return route.fulfill({ json: { username: "tester", csrf: "csrf" } });
    }
    if (path === "/settings") {
      return route.fulfill({
        json: {
          data: {
            ui_language: "ja",
            setup_complete: true,
            default_model_id: null,
          },
        },
      });
    }
    if (path === "/conversations" && request.method() === "GET") {
      const offset = Number(url.searchParams.get("offset") ?? "0");
      // Full first page, then a short last page: the client must ask for the
      // second offset and stop when the page is short.
      const count = offset === 0 ? CONVERSATION_PAGE_SIZE : 1;
      return route.fulfill({
        json: Array.from({ length: count }, (_, i) => row(offset + i)),
      });
    }
    return route.fulfill({ json: [] });
  });

  await page.goto("/");

  const loadMore = page.getByRole("button", {
    name: "以前の会話を読み込む",
  });
  await expect(loadMore).toBeVisible();
  await expect(page.getByText("会話 0", { exact: true })).toBeVisible();
  await expect(
    page.getByText(`会話 ${CONVERSATION_PAGE_SIZE}`, { exact: true }),
  ).toHaveCount(0);

  await loadMore.click();

  await expect(
    page.getByText(`会話 ${CONVERSATION_PAGE_SIZE}`, { exact: true }),
  ).toBeVisible();
  // The short last page ends the list, so the control disappears.
  await expect(loadMore).toHaveCount(0);
});
