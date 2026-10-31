import { describe, it, expect, afterEach } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { WindowControls } from "./WindowControls";

describe("WindowControls", () => {
  const originalPlatform = document.documentElement.dataset.platform;

  afterEach(() => {
    if (originalPlatform !== undefined) {
      document.documentElement.dataset.platform = originalPlatform;
    } else {
      delete document.documentElement.dataset.platform;
    }
  });

  it("does not render when platform is macos", () => {
    document.documentElement.dataset.platform = "macos";
    const { container } = render(<WindowControls />);
    expect(container.firstChild).toBeNull();
  });

  it("hides the buttons when plugin-os corrects the platform to macOS", async () => {
    // main.tsx writes a UA guess ("other") first; plugin-os is authoritative
    // and replaces it after mount.
    document.documentElement.dataset.platform = "other";
    const { container } = render(<WindowControls />);
    expect(container.firstChild).not.toBeNull();

    await act(async () => {
      document.documentElement.dataset.platform = "macos";
    });
    expect(container.firstChild).toBeNull();
  });

  it("renders window control buttons on windows/other platform", () => {
    document.documentElement.dataset.platform = "other";
    render(<WindowControls />);

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
});
