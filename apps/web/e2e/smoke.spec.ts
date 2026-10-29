import { expect, test } from "@playwright/test";

/**
 * Deterministic smoke test for the first-run surface.
 *
 * The API is mocked because the Docker edition needs PostgreSQL; the point of
 * this test is the React boot path (setup probe -> admin form -> validation),
 * which needs no database and catches a broken bundle or a broken boot effect.
 */
test("first-run admin form renders and validates the password", async ({
  page,
}) => {
  await page.route("**/api/v1/setup", (route) =>
    route.fulfill({ json: { needs_admin: true } }),
  );
  await page.route("**/api/v1/auth/me", (route) =>
    route.fulfill({ status: 401, json: { detail: "unauthenticated" } }),
  );

  await page.goto("/");

  await expect(
    page.getByRole("heading", { name: "ようこそ、MIX agentへ。" }),
  ).toBeVisible();

  await page.locator('input[name="username"]').fill("tester");
  await page.locator('input[name="password"]').fill("test-password-12345");
  await page
    .locator('input[name="confirm_password"]')
    .fill("different-password");

  await expect(page.getByText("パスワードが一致しません")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "ワークスペースを作成" }),
  ).toBeDisabled();
});

test("login failure is shown in English via the stable error code", async ({
  page,
}) => {
  // The language is read before React boots, so seed it first.
  await page.addInitScript(() =>
    localStorage.setItem("mix-agent-language", "en"),
  );
  await page.route("**/api/v1/setup", (route) =>
    route.fulfill({ json: { needs_admin: false } }),
  );
  await page.route("**/api/v1/auth/me", (route) =>
    route.fulfill({
      status: 401,
      json: { detail: "ログインしてください", code: "auth.login_required" },
    }),
  );
  await page.route("**/api/v1/auth/login", (route) =>
    route.fulfill({
      status: 401,
      json: {
        detail: "ユーザー名またはパスワードが違います",
        code: "auth.invalid_credentials",
      },
    }),
  );

  await page.goto("/");
  await page.locator('input[name="username"]').fill("tester");
  await page.locator('input[name="password"]').fill("wrong-password-123");
  await page.getByRole("button", { name: "Sign in" }).click();

  await expect(page.getByText("Incorrect username or password.")).toBeVisible();
});
