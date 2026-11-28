import { expect, test, type Page, type Route } from "@playwright/test";

/** Set by `E2E_REAL_API=1`: the stack at E2E_BASE_URL supplies the API. */
const useRealApi = process.env.E2E_REAL_API === "1";

/**
 * Approval decision round-trip against the mocked API.
 *
 * The card is assembled from three endpoints (persisted tool call, run with
 * its approval list, and a closed SSE stream), so this test also pins that
 * contract. Each decision is posted to /approvals/{id}/decision, after which
 * the invalidation refetch serves a resolved snapshot and hides the card.
 */

type Decision = "once" | "always" | "denied";

interface ApprovalFixture {
  risk?: string;
  decided?: Decision;
}

function json(route: Route, body: unknown) {
  return route.fulfill({ json: body });
}

async function mockApprovalPage(page: Page, fixture: ApprovalFixture) {
  const decisions: { decision: Decision; body: unknown }[] = [];
  // The first-visit tour is a modal backdrop that would cover the card.
  await page.addInitScript(() =>
    localStorage.setItem("mix-agent-tour-done", "1"),
  );
  // E2E_REAL_API=1 leaves the stack's API in place; the decision assertions
  // below need the in-process spy and are skipped in that mode.
  if (useRealApi) return decisions;
  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace(/^\/api\/v1/, "");
    if (path === "/setup") return json(route, { needs_admin: false });
    if (path === "/auth/me")
      return json(route, { username: "tester", csrf: "csrf" });
    if (path === "/settings")
      return json(route, {
        data: {
          ui_language: "ja",
          setup_complete: true,
          default_model_id: null,
        },
      });
    if (path === "/providers")
      return json(route, [
        {
          id: "p1",
          data: { name: "Test provider", base_url: "https://example.com" },
          created_at: "2026-01-01T00:00:00Z",
        },
      ]);
    if (path === "/models")
      return json(route, [
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
      ]);
    if (path === "/conversations/c1")
      return json(route, {
        id: "c1",
        data: { title: "承認テスト", pinned: false },
        created_at: "2026-01-01T00:00:00Z",
      });
    if (path === "/conversations/c1/messages")
      return json(route, {
        messages: [
          {
            id: "m1",
            data: { role: "user", content: "コマンドを実行して" },
            created_at: "2026-01-01T00:00:00Z",
          },
        ],
        runs: [{ id: "r1", status: "waiting_approval", message_id: "m1" }],
        selection: null,
        project_id: null,
        todos: [],
      });
    if (path === "/conversations/c1/tool-calls")
      return json(route, [
        {
          id: "call-1",
          run_id: "r1",
          status: fixture.decided ? "completed" : "waiting_approval",
          tool_name: "run_terminal",
          created_at: "2026-01-01T00:00:00Z",
          ...(fixture.decided
            ? {}
            : { approval: { id: "ap-1", status: "pending" } }),
          retry: { available: false, label: "" },
        },
      ]);
    if (path === "/runs/r1/events")
      return route.fulfill({
        status: 200,
        contentType: "text/event-stream",
        body: "event: done\ndata: {}\n\n",
      });
    if (path === "/runs/r1")
      return json(route, {
        id: "r1",
        status: fixture.decided ? "completed" : "waiting_approval",
        message_id: "m1",
        approvals: [
          {
            id: "ap-1",
            status: fixture.decided ? "once" : "pending",
            created_at: "2026-01-01T00:00:00Z",
            data: {
              tool: "run_terminal",
              arguments: { command: "echo hello" },
              scope: { workspace: "/workspace" },
              ...(fixture.risk ? { risk: fixture.risk } : {}),
            },
          },
        ],
        budget: {
          max_seconds: 300,
          max_steps: 20,
          max_tool_calls: 20,
          started_at: "2026-01-01T00:00:00Z",
        },
        remaining: {
          max_seconds: 300,
          max_steps: 20,
          max_tool_calls: 20,
          started_at: "2026-01-01T00:00:00Z",
        },
        policy: {},
        mode: "chat",
        steps: 1,
        tool_count: 1,
        browser_manual_active: false,
        browser_pause_requested: false,
      });
    if (path === "/approvals/ap-1/decision" && request.method() === "POST") {
      decisions.push({
        decision: (request.postDataJSON() as { decision: Decision }).decision,
        body: request.postDataJSON(),
      });
      return json(route, { ok: true });
    }
    return json(route, []);
  });

  await page.goto("/chat/c1");
  await expect(page.getByText("承認が必要です")).toBeVisible();
  return decisions;
}

test("once decision is posted and the card dismisses after refetch", async ({
  page,
}) => {
  test.skip(
    useRealApi,
    "the posted decision is observed by the mocked route handler",
  );
  const fixture: ApprovalFixture = { risk: "read" };
  const decisions = await mockApprovalPage(page, fixture);

  await page.getByRole("button", { name: "今回のみ" }).click();
  await expect.poll(() => decisions.length).toBe(1);
  expect(decisions[0]).toEqual({
    decision: "once",
    body: { decision: "once" },
  });
  fixture.decided = "once";
  await expect(page.getByText("承認が必要です")).toHaveCount(0);
});

test("always and deny decisions carry the right payload", async ({ page }) => {
  test.skip(
    useRealApi,
    "the posted decisions are observed by the mocked route handler",
  );
  const decisions = await mockApprovalPage(page, { risk: "read" });
  await page.getByRole("button", { name: "この操作を常に許可" }).click();
  await expect.poll(() => decisions.length).toBe(1);
  expect(decisions[0]).toEqual({
    decision: "always",
    body: { decision: "always" },
  });

  await page.reload();
  await expect(page.getByText("承認が必要です")).toBeVisible();
  await page.getByRole("button", { name: "拒否" }).click();
  await expect.poll(() => decisions.length).toBe(2);
  expect(decisions[1]).toEqual({
    decision: "denied",
    body: { decision: "denied" },
  });
});

test("risky grant carries acknowledgeRisk and is blocked until ticked", async ({
  page,
}) => {
  test.skip(
    useRealApi,
    "the posted decisions are observed by the mocked route handler",
  );
  const decisions = await mockApprovalPage(page, { risk: "write" });
  await expect(
    page.getByRole("button", { name: "この操作を常に許可" }),
  ).toHaveCount(0);
  const once = page.getByRole("button", { name: "今回のみ" });
  await expect(once).toBeDisabled();

  // Even a forced DOM click on the disabled button must not submit a grant.
  await once.evaluate((el: HTMLButtonElement) => el.click());
  expect(decisions).toHaveLength(0);

  const checkbox = page.getByRole("checkbox", {
    name: "内容を理解しました。この操作を実行します。",
  });
  await checkbox.check();
  await expect(once).toBeEnabled();
  await once.click();
  await expect.poll(() => decisions.length).toBe(1);
  expect(decisions[0].body).toEqual({
    decision: "once",
    acknowledgeRisk: true,
  });

  // Denying a risky approval needs no acknowledgement and stays a plain body.
  await page.reload();
  await expect(page.getByText("承認が必要です")).toBeVisible();
  await page.getByRole("button", { name: "拒否" }).click();
  await expect.poll(() => decisions.length).toBe(2);
  expect(decisions[1].body).toEqual({ decision: "denied" });
});

test("approval controls have accessible labels and no dangling aria references", async ({
  page,
}) => {
  test.skip(
    useRealApi && !process.env.E2E_BASE_URL,
    "E2E_REAL_API=1 needs E2E_BASE_URL pointing at a running stack",
  );
  await mockApprovalPage(page, { risk: "write" });
  const audit = await page.evaluate(() => {
    const card = document.querySelector(".run-step-approval");
    if (!card) return { error: "card missing" };
    const buttons = Array.from(card.querySelectorAll("button"));
    const emptyButtons = buttons.filter(
      (b) => !(b.textContent ?? "").trim() && !b.getAttribute("aria-label"),
    ).length;
    const checkbox = card.querySelector('input[type="checkbox"]');
    const labelled =
      !!checkbox &&
      (!!checkbox.closest("label") || !!checkbox.getAttribute("aria-label"));
    const ids = Array.from(card.querySelectorAll("[id]")).map((n) => n.id);
    const dangling = Array.from(card.querySelectorAll("[aria-describedby]"))
      .flatMap((n) => (n.getAttribute("aria-describedby") ?? "").split(/\s+/))
      .filter((id) => id && !document.getElementById(id));
    return {
      emptyButtons,
      labelled,
      duplicateIds: ids.length - new Set(ids).size,
      dangling,
    };
  });
  expect(audit).toEqual({
    emptyButtons: 0,
    labelled: true,
    duplicateIds: 0,
    dangling: [],
  });
});
