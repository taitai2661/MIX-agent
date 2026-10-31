import { expect, test } from "@playwright/test";

/**
 * Chat round-trip against a mocked API.
 *
 * The smoke suite covers boot and login. This one exercises the path that
 * actually breaks when the run-event contract moves: send a message, attach
 * the SSE stream, render streamed text, and finish on `done`. No database or
 * provider is needed; every `/api/v1` call is answered in-process.
 */
test("send renders streamed assistant text and finishes on done", async ({ page }) => {
  const events = [
    'event: activity\ndata: {"kind":"text","text":"返答です"}\n\n',
    "event: done\ndata: {}\n\n",
  ].join("");

  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace(/^\/api\/v1/, "");

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
    if (path === "/conversations/c1/messages" && request.method() === "POST") {
      return route.fulfill({ json: { run_id: "r1" } });
    }
    if (path === "/conversations/c1/messages") {
      return route.fulfill({
        json: {
          messages: [
            {
              id: "m1",
              data: { role: "user", content: "こんにちは" },
              created_at: "2026-01-01T00:00:00Z",
            },
          ],
          runs: [],
          selection: null,
          project_id: null,
          todos: [],
        },
      });
    }
    if (path === "/runs/r1/events") {
      return route.fulfill({
        status: 200,
        contentType: "text/event-stream",
        body: events,
      });
    }
    if (path === "/runs/r1") {
      return route.fulfill({
        json: {
          id: "r1",
          status: "completed",
          message_id: "m1",
          data: {},
          steps: 1,
        },
      });
    }
    if (path === "/providers") {
      return route.fulfill({
        json: [
          {
            id: "p1",
            data: { name: "Test provider", base_url: "https://example.com" },
            created_at: "2026-01-01T00:00:00Z",
          },
        ],
      });
    }
    if (path === "/models") {
      return route.fulfill({
        json: [
          {
            id: "model-1",
            data: {
              provider_id: "p1",
              model_id: "test-model",
              name: "Test model",
              capabilities: {},
            },
            created_at: "2026-01-01T00:00:00Z",
          },
        ],
      });
    }
    if (
      path.startsWith("/conversations") ||
      path.startsWith("/tools") ||
      path.startsWith("/agents") ||
      path.startsWith("/projects") ||
      path.startsWith("/permission-rules") ||
      path.startsWith("/conversation-folders")
    ) {
      return route.fulfill({ json: [] });
    }
    return route.fulfill({ json: [] });
  });

  await page.goto("/chat/c1");

  const composer = page.getByPlaceholder("MIX agent にメッセージを送信…");
  await expect(composer).toBeVisible();
  await composer.fill("こんにちは");
  await composer.press("Enter");

  // The user message survives the post-send refetch, and the SSE `text` event
  // is rendered as the assistant's answer.
  await expect(page.getByText("こんにちは").first()).toBeVisible();
  await expect(page.getByText("返答です")).toBeVisible();
});
