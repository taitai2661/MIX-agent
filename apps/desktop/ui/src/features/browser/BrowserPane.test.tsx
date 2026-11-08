import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { setLanguage } from "@/lib/i18n";
import { api } from "@/lib/api";
import { useUi } from "@/lib/ui";
import { BrowserPane } from "./BrowserPane";

vi.mock("@/lib/api", () => ({
  api: {
    browserClearData: vi.fn(async () => undefined),
  },
}));

const mockedApi = vi.mocked(api);
const WORKSPACE = "11111111-1111-4111-8111-111111111111";

beforeAll(() => setLanguage("ja"));

beforeEach(() => {
  vi.clearAllMocks();
  useUi.setState({ workspaceId: WORKSPACE });
});

describe("BrowserPane", () => {
  it("explains that the page is a native WebView", async () => {
    render(<BrowserPane />);
    expect(
      await screen.findByText(/ネイティブ WebView で表示されます/),
    ).toBeInTheDocument();
  });

  it("clears browsing data after confirmation", async () => {
    const user = userEvent.setup();
    render(<BrowserPane />);
    await user.click(
      await screen.findByRole("button", { name: "閲覧データを削除" }),
    );
    await user.click(await screen.findByRole("button", { name: "削除" }));
    await waitFor(() =>
      expect(mockedApi.browserClearData).toHaveBeenCalledWith(WORKSPACE),
    );
  });

  it("disables the action without a workspace", async () => {
    useUi.setState({ workspaceId: null });
    render(<BrowserPane />);
    expect(
      await screen.findByRole("button", { name: "閲覧データを削除" }),
    ).toBeDisabled();
  });
});
