import { beforeAll, describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { setLanguage } from "@/lib/i18n";
import type { MessageView } from "@/lib/types";
import { Transcript } from "./Transcript";

function userMessage(content: string): MessageView {
  return {
    id: "m-1",
    role: "user",
    content,
    reasoning: null,
    model_id: "p:model",
    run_id: "run-1",
    kind: "user",
    created_at: new Date().toISOString(),
    attachments: [],
  };
}

// jsdom reports en-US; pin the language so the labels are deterministic.
beforeAll(() => setLanguage("ja"));

describe("Transcript user-message editing", () => {
  it("edits a user turn in place and hands the new text to the caller", async () => {
    const onEditMessage = vi.fn().mockResolvedValue(undefined);
    const user = userEvent.setup();
    render(
      <Transcript
        messages={[userMessage("元のメッセージ")]}
        runs={new Map()}
        live={null}
        liveRunId={null}
        onEditMessage={onEditMessage}
      />,
    );

    await user.click(screen.getByRole("button", { name: "メッセージを編集" }));
    const box = screen.getByRole("textbox", { name: "メッセージを編集" });
    // The editor opens with the persisted text, not an empty draft.
    expect(box).toHaveValue("元のメッセージ");

    await user.clear(box);
    await user.type(box, "直したメッセージ");
    await user.click(screen.getByRole("button", { name: "保存" }));

    expect(onEditMessage).toHaveBeenCalledWith("m-1", "直したメッセージ");
    // On success the editor closes and the read-only bubble comes back.
    expect(screen.queryByRole("textbox", { name: "メッセージを編集" })).toBeNull();
    expect(screen.getByText("元のメッセージ")).toBeInTheDocument();
  });

  it("discards the draft on cancel without calling the caller", async () => {
    const onEditMessage = vi.fn().mockResolvedValue(undefined);
    const user = userEvent.setup();
    render(
      <Transcript
        messages={[userMessage("元のメッセージ")]}
        runs={new Map()}
        live={null}
        liveRunId={null}
        onEditMessage={onEditMessage}
      />,
    );

    await user.click(screen.getByRole("button", { name: "メッセージを編集" }));
    await user.clear(screen.getByRole("textbox", { name: "メッセージを編集" }));
    await user.type(
      screen.getByRole("textbox", { name: "メッセージを編集" }),
      "破棄される",
    );
    await user.click(screen.getByRole("button", { name: "キャンセル" }));

    expect(onEditMessage).not.toHaveBeenCalled();
    expect(screen.getByText("元のメッセージ")).toBeInTheDocument();
  });

  it("hides the affordance while a run is in flight", () => {
    render(
      <Transcript
        messages={[userMessage("元のメッセージ")]}
        runs={new Map()}
        live={null}
        liveRunId={null}
        onEditMessage={() => Promise.resolve()}
        editDisabled
      />,
    );
    expect(screen.getByRole("button", { name: "メッセージを編集" })).toBeDisabled();
  });

  it("keeps the editor open when the caller rejects", async () => {
    const onEditMessage = vi.fn().mockRejectedValue(new Error("実行中"));
    const user = userEvent.setup();
    render(
      <Transcript
        messages={[userMessage("元のメッセージ")]}
        runs={new Map()}
        live={null}
        liveRunId={null}
        onEditMessage={onEditMessage}
      />,
    );

    await user.click(screen.getByRole("button", { name: "メッセージを編集" }));
    await user.clear(screen.getByRole("textbox", { name: "メッセージを編集" }));
    await user.type(
      screen.getByRole("textbox", { name: "メッセージを編集" }),
      "残したい",
    );
    await user.click(screen.getByRole("button", { name: "保存" }));

    expect(onEditMessage).toHaveBeenCalled();
    // The typed text survives a failed save instead of being thrown away.
    expect(screen.getByRole("textbox", { name: "メッセージを編集" })).toHaveValue(
      "残したい",
    );
  });
});
