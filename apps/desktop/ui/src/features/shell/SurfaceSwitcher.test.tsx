import { afterEach, beforeAll, beforeEach, describe, expect, it } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { setLanguage } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { SurfaceSwitcher } from "./SurfaceSwitcher";

beforeAll(() => setLanguage("ja"));

describe("SurfaceSwitcher", () => {
  beforeEach(() => {
    localStorage.clear();
    useUi.setState({ surface: "chat", mode: "chat", threadFilter: "all" });
  });
  afterEach(() => {
    localStorage.clear();
    useUi.setState({ surface: "chat", mode: "chat", threadFilter: "all" });
  });

  it("shows the active surface label in the trigger", () => {
    render(<SurfaceSwitcher />);
    expect(screen.getByTestId("surface-switcher-trigger")).toHaveTextContent(
      "チャット",
    );
  });

  it("reflects a switch to the desktop-agent surface", () => {
    act(() => {
      useUi.setState({ surface: "desktop-agent", mode: "agent" });
    });
    render(<SurfaceSwitcher />);
    expect(screen.getByTestId("surface-switcher-trigger")).toHaveTextContent(
      "コーディング",
    );
  });

  it("switching surfaces snaps the run mode to one the surface allows", async () => {
    act(() => {
      useUi.setState({ surface: "chat", mode: "thinking" });
    });
    const user = userEvent.setup();
    render(<SurfaceSwitcher />);
    await user.click(screen.getByTestId("surface-switcher-trigger"));
    await user.click(screen.getByTestId("surface-switcher-item-desktop-agent"));
    expect(useUi.getState().surface).toBe("desktop-agent");
    // Mode was "thinking" which desktop-agent does not allow, so it snaps to "agent".
    expect(useUi.getState().mode).toBe("agent");
  });

  it("switching back to chat from desktop-agent keeps the chat-default mode", async () => {
    act(() => {
      useUi.setState({ surface: "desktop-agent", mode: "agent" });
    });
    const user = userEvent.setup();
    render(<SurfaceSwitcher />);
    await user.click(screen.getByTestId("surface-switcher-trigger"));
    await user.click(screen.getByTestId("surface-switcher-item-chat"));
    expect(useUi.getState().surface).toBe("chat");
    expect(useUi.getState().mode).toBe("chat");
  });
});
