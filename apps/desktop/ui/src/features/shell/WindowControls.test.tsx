import { describe, it, expect, afterEach, vi } from "vitest";
import { act, cleanup, render, screen } from "@testing-library/react";
import { WindowControls } from "./WindowControls";

// The component asks the native window whether the OS is already drawing a
// title bar. Stand in for the Tauri window API so both branches are testable;
// without this the dynamic import fails and every test only covers the
// browser-preview fallback.
const windowState = { decorated: false, maximized: false };

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isMaximized: async () => windowState.maximized,
    isDecorated: async () => windowState.decorated,
    onResized: async () => () => {},
    minimize: async () => {},
    toggleMaximize: async () => {},
    close: async () => {},
  }),
}));

/** Let the async `isDecorated` / `onResized` setup settle inside `act`. */
async function flushWindowQuery() {
  await act(async () => {});
}

describe("WindowControls", () => {
  const originalPlatform = document.documentElement.dataset.platform;

  afterEach(() => {
    // Unmount before touching `data-platform`: the component watches that
    // attribute, and a late observer callback would be an un-acted state
    // update in the next test.
    cleanup();
    windowState.decorated = false;
    windowState.maximized = false;
    if (originalPlatform !== undefined) {
      document.documentElement.dataset.platform = originalPlatform;
    } else {
      delete document.documentElement.dataset.platform;
    }
  });

  it("does not render when platform is macos", async () => {
    document.documentElement.dataset.platform = "macos";
    const { container } = render(<WindowControls />);
    await flushWindowQuery();
    expect(container.firstChild).toBeNull();
  });

  it("hides the buttons when plugin-os corrects the platform to macOS", async () => {
    // main.tsx writes a UA guess ("other") first; plugin-os is authoritative
    // and replaces it after mount.
    document.documentElement.dataset.platform = "other";
    const { container } = render(<WindowControls />);
    await flushWindowQuery();
    expect(container.firstChild).not.toBeNull();

    await act(async () => {
      document.documentElement.dataset.platform = "macos";
    });
    expect(container.firstChild).toBeNull();
  });

  it("renders window control buttons on an undecorated windows/other window", async () => {
    document.documentElement.dataset.platform = "other";
    windowState.decorated = false;
    render(<WindowControls />);
    await flushWindowQuery();

    expect(
      screen.getByRole("toolbar", { name: /ウィンドウ操作|window controls/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /最小化|minimize/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /最大化|maximize/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /閉じる|close/i }),
    ).toBeInTheDocument();
  });

  it("stays hidden when the OS already draws the window buttons", async () => {
    // Windows/Linux builds normally run undecorated, but an older installer
    // (or a config that skipped the platform override) can still show the
    // native title bar. Our three buttons would then be a visible duplicate.
    document.documentElement.dataset.platform = "windows";
    windowState.decorated = true;
    const { container } = render(<WindowControls />);
    await flushWindowQuery();

    expect(container.firstChild).toBeNull();
    expect(screen.queryByRole("toolbar")).toBeNull();
  });
});
