import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { setLanguage } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { BrowserPanel } from "./BrowserPanel";
import { normalizeAddress, displayAddress } from "./BrowserToolbar";

vi.mock("@/lib/api", () => ({
  api: {
    browserStatus: vi.fn(async () => ({
      backend: "chromium",
      source: "installed",
      executable: "/tmp/chromium",
      installed_version: "156.0.8078.73",
      pinned_version: "156.0.8078.73",
      update_available: false,
      install: {
        status: "ready",
        progress: 100,
        stage: "done",
        failure: null,
        failure_code: null,
        bytes_done: null,
        bytes_total: null,
        version: "156.0.8078.73",
      },
      profiles: 1,
      artifacts: 0,
      allow_local: false,
    })),
    browserProfiles: vi.fn(async () => []),
    browserInvoke: vi.fn(async () => ({
      ok: true,
      payload: {
        active: { url: "https://example.com/", title: "Example" },
        pages: [
          {
            id: "tab-1",
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
    browserFrame: vi.fn(async () => ({
      data: "data:image/jpeg;base64,AA==",
      width: 1280,
      height: 900,
      url: "https://example.com/",
      title: "Example",
      can_go_back: false,
      can_go_forward: false,
    })),
    browserResetProfile: vi.fn(async () => undefined),
    browserInstall: vi.fn(),
    browserInstallCancel: vi.fn(),
    browserUninstall: vi.fn(),
    browserInstallEvent: vi.fn(async () => () => undefined),
    browserArtifact: vi.fn(async () => "data:image/png;base64,AA=="),
    browserStop: vi.fn(async () => undefined),
    browserKeepalive: vi.fn(async () => undefined),
    browserScreencastStart: vi.fn(async () => undefined),
    browserScreencastStop: vi.fn(async () => undefined),
    browserFrameEvent: vi.fn(async () => () => undefined),
    openExternal: vi.fn(async () => undefined),
  },
}));

import { api } from "@/lib/api";

const mockedApi = vi.mocked(api);
const WORKSPACE = "11111111-1111-4111-8111-111111111111";

beforeAll(() => setLanguage("ja"));

beforeEach(() => {
  vi.clearAllMocks();
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
  it("auto-starts live view for an agent-opened pane", async () => {
    render(<BrowserPanel />);
    expect(
      await screen.findByAltText("ブラウザのライブ画面"),
    ).toBeInTheDocument();
    expect(mockedApi.browserScreencastStart).toHaveBeenCalledWith(WORKSPACE);
  });

  it("opens the install path when Chromium is missing", async () => {
    mockedApi.browserStatus.mockResolvedValueOnce({
      backend: "disabled",
      source: "none",
      executable: null,
      installed_version: null,
      pinned_version: "156.0.8078.73",
      update_available: false,
      install: {
        status: "not_installed",
        progress: null,
        stage: null,
        failure: null,
        failure_code: null,
        bytes_done: null,
        bytes_total: null,
        version: null,
      },
      profiles: 0,
      artifacts: 0,
      allow_local: false,
    });
    render(<BrowserPanel />);
    expect(
      await screen.findByRole("button", { name: /Chromium をインストール/ }),
    ).toBeInTheDocument();
    expect(mockedApi.browserScreencastStart).not.toHaveBeenCalled();
  });

  it("shows the thread prompt without a workspace", async () => {
    useUi.setState({ workspaceId: null });
    render(<BrowserPanel />);
    expect(
      await screen.findByText("スレッドを開始するとブラウザを使えます。"),
    ).toBeInTheDocument();
  });

  it("does not auto-start for a user-opened pane", async () => {
    useUi.setState({ browserOpenedBy: "user" });
    render(<BrowserPanel />);
    expect(
      await screen.findByText(
        "ライブ表示を開始すると、ブラウザ画面をここで確認できます。",
      ),
    ).toBeInTheDocument();
    expect(mockedApi.browserScreencastStart).not.toHaveBeenCalled();
  });

  it("waits for status before deciding the pane is missing Chromium", async () => {
    render(<BrowserPanel />);
    await waitFor(() => expect(mockedApi.browserStatus).toHaveBeenCalled());
    expect(
      screen.queryByText(
        "ブラウザ機能を使うにはChromiumをインストールしてください。約200MBをダウンロードします。",
      ),
    ).toBeNull();
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
