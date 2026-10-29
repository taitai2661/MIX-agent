import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { Tour } from "./tour";

it("renders the first onboarding step with navigation controls", () => {
  const html = renderToStaticMarkup(<Tour onClose={() => {}} />);
  expect(html).toContain("チャット");
  expect(html).toContain("次へ");
  expect(html).toContain("スキップ");
});
