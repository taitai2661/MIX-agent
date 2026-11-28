import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ModelView } from "@/lib/types";
import { useComposerState } from "./useComposerState";

const mocked = vi.hoisted(() => ({
  attachments: [] as Array<{
    kind: "image";
    mime_type: string;
    data_base64: string;
    filename: string;
  }>,
  setModelId: vi.fn(),
}));

vi.mock("./useAttachments", () => ({
  useAttachments: () => ({
    attachments: mocked.attachments,
    attachError: null,
    attaching: false,
    addImageFiles: vi.fn(async () => {}),
    onAttach: vi.fn(async () => {}),
    captureScreenshot: vi.fn(async () => false),
    screenshotAvailable: false,
    removeAttachment: vi.fn(),
    removeAttachments: vi.fn(),
    clearAttachments: vi.fn(),
  }),
}));

vi.mock("./useVoiceInput", () => ({
  useVoiceInput: () => ({
    listening: false,
    toggleVoiceInput: vi.fn(),
    speechSupported: false,
  }),
}));

function model(id: string, supports_vision: boolean): ModelView {
  return {
    id,
    provider_id: "p",
    label: id,
    context_window: 128000,
    supports_vision,
    supports_tool_calling: true,
    supports_reasoning: true,
    auto_candidate: true,
  };
}

const textModel = model("p:text", false);
const manualModel = model("p:manual", false);

function Harness({ withImage }: { withImage: boolean }) {
  // Mimic Composer + the UI store: the model id is state owned above the hook,
  // so `setModelId("auto")` really changes the props the hook sees next render.
  const [modelId, setModelIdState] = useState("p:text");
  const setModelId = (id: string) => {
    mocked.setModelId(id);
    setModelIdState(id);
  };
  const state = useComposerState({
    setMode: () => {},
    draftKey: "chat-1",
    rootPath: null,
    sendShortcut: "modifier",
    busy: false,
    modelId,
    setModelId,
    selectedModel:
      modelId === "auto"
        ? undefined
        : modelId === "p:manual"
          ? manualModel
          : textModel,
    staleModel: false,
    onSend: async () => {},
  });
  return (
    <div data-image={withImage}>
      <span data-testid="active">{String(state.autoVisionActive)}</span>
      <button
        type="button"
        onClick={() => {
          mocked.setModelId.mockClear();
          setModelIdState("p:manual");
        }}
      >
        manual
      </button>
    </div>
  );
}

const image = {
  kind: "image" as const,
  mime_type: "image/png",
  data_base64: "eA==",
  filename: "a.png",
};

beforeEach(() => {
  mocked.attachments = [];
  mocked.setModelId.mockClear();
  localStorage.clear();
});

describe("useComposerState auto vision", () => {
  it("does not write 'auto' back over a manual switch made while an image is attached", () => {
    mocked.attachments = [image];
    const { rerender } = render(<Harness withImage />);
    expect(mocked.setModelId).toHaveBeenCalledWith("auto");
    expect(screen.getByTestId("active").textContent).toBe("true");

    // Leaving `auto` for another explicit model must not be reverted. The
    // opt-out effect is declared before the auto-switch effect so it wins the
    // same-commit race against a stale `autoOptOutRef`.
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "manual" }));
    });
    expect(mocked.setModelId).not.toHaveBeenCalledWith("auto");
    expect(screen.getByTestId("active").textContent).toBe("false");

    // Removing the image must not restore the pre-switch model either.
    mocked.attachments = [];
    rerender(<Harness withImage={false} />);
    expect(mocked.setModelId).not.toHaveBeenCalled();
  });
});
