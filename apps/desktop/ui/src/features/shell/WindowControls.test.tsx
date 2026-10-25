import { describe, it, expect, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
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
