import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { setLanguage } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { AccentPicker } from "./accent";

describe("AccentPicker", () => {
  beforeEach(() => {
    localStorage.clear();
    setLanguage("ja");
    document.documentElement.dataset.theme = "light";
    useUi.setState({ accent: "blue" });
  });

  afterEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute("data-theme");
    document.documentElement.removeAttribute("style");
  });

  it("renders every accent and applies the selected one", async () => {
    const user = userEvent.setup();
    render(<AccentPicker />);

    expect(screen.getAllByRole("button")).toHaveLength(7);
    const rose = screen.getByRole("button", { name: "ピンク" });
    expect(rose).toHaveAttribute("aria-pressed", "false");

    await user.click(rose);

    expect(rose).toHaveAttribute("aria-pressed", "true");
    expect(useUi.getState().accent).toBe("rose");
    expect(localStorage.getItem("mix-agent-accent")).toBe("rose");
    expect(
      document.documentElement.style.getPropertyValue("--color-brand"),
    ).toBe("#db2777");
  });
});
