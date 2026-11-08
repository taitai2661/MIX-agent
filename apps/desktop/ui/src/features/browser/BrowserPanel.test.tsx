import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { setLanguage } from "@/lib/i18n";
import { api } from "@/lib/api";
import { useUi } from "@/lib/ui";
import { BrowserPanel } from "./BrowserPanel";
import { displayAddress, normalizeAddress } from "./BrowserToolbar";

vi.mock("@/lib/api", () => ({
  api: {
    browserInvoke: vi.fn(async () => ({
      ok: true,
      payload: {
        active: { id: "t1", url: "https://example.com/", title: "Example" },
        pages: [
          {
            id: "t1",
            url: "https://example.com/",
            title: "Example",
            active: true,
          },
        ],
        can_go_back: false,
        can_go_forward: false,
      },
      artifact_id: null,
      error: null,
    })),
    browserSurfaceBounds: vi.fn(async () => undefined),
    browserStateEvent: vi.fn(async () => () => undefined),
    browserStop: vi.fn(async () => undefined),
    browserClearData: vi.fn(async () => undefined),
    openExternal: vi.fn(async () => undefined),
  },
}));

const mockedApi = vi.mocked(api);
const WORKSPACE = "11111111-1111-4111-8111-111111111111";

beforeAll(() => setLanguage("ja"));

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    x: 10,
    y: 10,
    width: 600,
    height: 400,
    top: 10,
    left: 10,
    right: 610,
    bottom: 410,
    toJSON: () => ({}),
  } as DOMRect);
  useUi.setState({
    workspaceId: WORKSPACE,
    conversationId: null,
    rightPanel: "browser",
    sidePanelMode: "split",
    browserOpenedBy: "agent",
    browserAutoOpen: true,
  });
});

describe("BrowserPanel", () => {
  it("shows the native surface and the active tab", async () => {
    render(<BrowserPanel />);
    expect(await screen.findByText("Example")).toBeInTheDocument();
    // The title and the omnibox are fed by the same info refresh, but the
    // toolbar updates its address in a passive effect. Wait for that effect
    // instead of asserting immediately after the title appears: under a loaded
    // CI runner the DOM mutation can be observed before passive effects flush.
    const addressBar = screen.getByLabelText("アドレスバー");
    await waitFor(() => expect(addressBar).toHaveValue("example.com"));
    expect(
      screen.getByText("ネイティブブラウザを読み込んでいます…"),
    ).toBeInTheDocument();
    expect(mockedApi.browserInvoke).toHaveBeenCalledWith(
      expect.objectContaining({ action: "info" }),
    );
  });

  it("shows the thread prompt without a workspace", async () => {
    useUi.setState({ workspaceId: null });
    render(<BrowserPanel />);
    expect(
      await screen.findByText("スレッドを開始するとブラウザを使えます。"),
    ).toBeInTheDocument();
  });

  it("reports the surface rectangle to Rust", async () => {
    render(<BrowserPanel />);
    await screen.findByText("Example");
    await waitFor(() =>
      expect(mockedApi.browserSurfaceBounds).toHaveBeenCalledWith(
        WORKSPACE,
        expect.objectContaining({ visible: true }),
      ),
    );
  });
});

describe("address normalization", () => {
  it("keeps explicit schemes", () => {
    expect(normalizeAddress("https://example.com/a")).toBe(
      "https://example.com/a",
    );
    expect(normalizeAddress("about:blank")).toBe("about:blank");
  });

  it("uses http for loopback and https otherwise", () => {
    expect(normalizeAddress("localhost:3000")).toBe("http://localhost:3000");
    expect(normalizeAddress("127.0.0.1:8080/x")).toBe(
      "http://127.0.0.1:8080/x",
    );
    expect(normalizeAddress("example.com/docs")).toBe(
      "https://example.com/docs",
    );
  });

  it("renders a compact display address", () => {
    expect(displayAddress("https://example.com/a?b=1")).toBe(
      "example.com/a?b=1",
    );
    expect(displayAddress("about:blank")).toBe("about:blank");
  });
});
