import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/api";
import { MessageFeedback } from "./MessageFeedback";

vi.mock("@/lib/api", () => ({
  api: { setMessageFeedback: vi.fn(async () => "up") },
}));

const feedback = vi.mocked(api.setMessageFeedback);

describe("MessageFeedback", () => {
  beforeEach(() => {
    feedback.mockReset();
    feedback.mockResolvedValue("up");
  });

  it("sends up/down and clears on a second click", async () => {
    const user = userEvent.setup();
    render(<MessageFeedback messageId="m-1" initial={null} />);

    const up = screen.getByRole("button", { name: /高く評価|highly/i });
    const down = screen.getByRole("button", { name: /低く評価|poorly/i });

    await user.click(up);
    expect(feedback).toHaveBeenCalledWith("m-1", "up");
    expect(up).toHaveAttribute("aria-pressed", "true");

    await user.click(down);
    expect(feedback).toHaveBeenLastCalledWith("m-1", "down");
    expect(down).toHaveAttribute("aria-pressed", "true");
    expect(up).toHaveAttribute("aria-pressed", "false");

    // Clicking the active rating again clears it (null deletes the row).
    await user.click(down);
    expect(feedback).toHaveBeenLastCalledWith("m-1", null);
    expect(down).toHaveAttribute("aria-pressed", "false");
  });

  it("reverts the optimistic value when the write fails", async () => {
    const user = userEvent.setup();
    feedback.mockRejectedValueOnce(new Error("nope"));
    render(<MessageFeedback messageId="m-2" initial={null} />);

    const up = screen.getByRole("button", { name: /高く評価|highly/i });
    await user.click(up);

    await waitFor(() => expect(feedback).toHaveBeenCalledWith("m-2", "up"));
    await waitFor(() => expect(up).toHaveAttribute("aria-pressed", "false"));
    // The failure is announced instead of silently dropping the click.
    expect(screen.getByRole("alert")).toHaveTextContent(
      /評価を保存できませんでした|Could not save/,
    );
  });

  it("adopts a rating that arrives with a refetch", () => {
    const { rerender } = render(
      <MessageFeedback messageId="m-3" initial={null} />,
    );
    const up = screen.getByRole("button", { name: /高く評価|highly/i });
    expect(up).toHaveAttribute("aria-pressed", "false");

    rerender(<MessageFeedback messageId="m-3" initial="up" />);
    expect(up).toHaveAttribute("aria-pressed", "true");
  });
});
