import { beforeAll, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { setLanguage } from "@/lib/i18n";
import { api } from "@/lib/api";
import { useUi } from "@/lib/ui";
import type { BrowserInstallState } from "@/lib/types";
import { BrowserPane } from "./BrowserPane";

vi.mock("@/lib/api", () => ({
  api: {
    browserStatus: vi.fn(async () => ({
      backend: "chromium",
      source: "installed",
      headless: true,
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
    browserProfiles: vi.fn(async () => [
      {
        workspace_id: "11111111-1111-4111-8111-111111111111",
        display_name: "default",
        directory: "/tmp/profile",
        provisioned: true,
      },
    ]),
    browserInvoke: vi.fn(async () => ({
      ok: true,
      payload: { count: 1, nodes: [{ ref: "r1", role: "button" }] },
      artifact_id: null,
      error: null,
    })),
    browserResetProfile: vi.fn(async () => undefined),
    browserInstall: vi.fn(),
    browserInstallCancel: vi.fn(),
    browserUninstall: vi.fn(),
    browserInstallEvent: vi.fn(async () => () => undefined),
    browserArtifact: vi.fn(async () => "data:image/png;base64,AA=="),
    browserFrame: vi.fn(async () => ({
      data: "data:image/png;base64,AA==",
      width: 1280,
      height: 900,
    })),
  },
}));

const mockedApi = vi.mocked(api);
const WORKSPACE = "11111111-1111-4111-8111-111111111111";

beforeAll(() => setLanguage("ja"));

function renderPane() {
  useUi.setState({ workspaceId: WORKSPACE });
  return render(<BrowserPane />);
}

describe("BrowserPane", () => {
  it("renders the backend and known profiles", async () => {
    renderPane();
    expect(await screen.findByText("chromium")).toBeInTheDocument();
    expect(mockedApi.browserProfiles).toHaveBeenCalled();
    expect(
      await screen.findByRole("option", { name: /default/ }),
    ).toBeInTheDocument();
  });

  it("sends a snapshot through the shared contract", async () => {
    const user = userEvent.setup();
    renderPane();
    await screen.findByText("chromium");
    await user.click(screen.getByRole("button", { name: "Snapshot" }));
    await user.click(screen.getByRole("button", { name: "実行" }));
    await waitFor(() =>
      expect(mockedApi.browserInvoke).toHaveBeenCalledWith(
        expect.objectContaining({
          workspace_id: WORKSPACE,
          action: "snapshot",
        }),
      ),
    );
  });

  it("sends ref-based typing instead of a guessed selector", async () => {
    const user = userEvent.setup();
    renderPane();
    await screen.findByText("chromium");
    await user.click(screen.getByRole("button", { name: "Type" }));
    await user.type(
      screen.getByLabelText("スナップショットの ref（例: r12）"),
      "r4",
    );
    await user.type(screen.getByLabelText("入力する文字"), "hello");
    await user.click(screen.getByRole("button", { name: "実行" }));
    await waitFor(() =>
      expect(mockedApi.browserInvoke).toHaveBeenCalledWith(
        expect.objectContaining({
          action: "type",
          ref: "r4",
          text: "hello",
        }),
      ),
    );
  });

  it("offers an install button when Chromium is missing", async () => {
    mockedApi.browserStatus.mockResolvedValueOnce({
      backend: "disabled",
      source: "none",
      headless: true,
      executable: null,
      installed_version: null,
      pinned_version: "156.0.8078.73",
      update_available: true,
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
    const user = userEvent.setup();
    renderPane();
    const install = await screen.findByRole("button", { name: "インストール" });
    await user.click(install);
    await waitFor(() =>
      expect(mockedApi.browserInstall).toHaveBeenCalledWith(false),
    );
  });

  it("shows a failed install and offers a retry", async () => {
    mockedApi.browserStatus.mockResolvedValueOnce({
      backend: "disabled",
      source: "none",
      headless: true,
      executable: null,
      installed_version: null,
      pinned_version: "156.0.8078.73",
      update_available: true,
      install: {
        status: "failed",
        progress: null,
        stage: null,
        failure: "checksum mismatch",
        failure_code: "checksum_mismatch",
        bytes_done: null,
        bytes_total: null,
        version: null,
      },
      profiles: 0,
      artifacts: 0,
      allow_local: false,
    });
    renderPane();
    expect(await screen.findByText("checksum mismatch")).toBeInTheDocument();
    expect(
      await screen.findByRole("button", { name: "再試行" }),
    ).toBeInTheDocument();
  });

  it("merges install progress events into the cached status", async () => {
    let emit: ((state: BrowserInstallState) => void) | undefined;
    mockedApi.browserInstallEvent.mockImplementationOnce(async (handler) => {
      emit = handler;
      return () => undefined;
    });
    renderPane();
    await screen.findByText("chromium");

    act(() => {
      emit?.({
        status: "downloading",
        progress: 42,
        stage: "downloading",
        failure: null,
        failure_code: null,
        bytes_done: 42,
        bytes_total: 100,
        version: "156.0.8078.73",
      });
    });

    expect(await screen.findByRole("progressbar")).toHaveAttribute(
      "aria-valuenow",
      "42",
    );
  });

  it("starts live view by polling a non-persisted frame", async () => {
    const user = userEvent.setup();
    renderPane();
    await screen.findByText("chromium");
    await user.click(screen.getByRole("button", { name: "ライブ表示" }));
    await waitFor(() =>
      expect(mockedApi.browserFrame).toHaveBeenCalledWith(WORKSPACE, false),
    );
    expect(
      await screen.findByAltText("ブラウザのライブ画面"),
    ).toBeInTheDocument();
  });
});
