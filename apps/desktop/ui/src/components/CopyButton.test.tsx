import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/api";
import { setLanguage } from "@/lib/i18n";
import { useToasts } from "@/lib/toast";
import { CodeBlock, extensionForLanguage } from "./CopyButton";

beforeAll(() => setLanguage("ja"));

vi.mock("@/lib/api", () => ({
  api: {
    pickSaveFile: vi.fn(async () => "/tmp/snippet.ts"),
    saveTextFile: vi.fn(async () => 10),
  },
}));

const pickSaveFile = vi.mocked(api.pickSaveFile);
const saveTextFile = vi.mocked(api.saveTextFile);

beforeEach(() => {
  vi.clearAllMocks();
  useToasts.setState({ toasts: [] });
});

describe("extensionForLanguage", () => {
  it("maps aliases and falls back to txt", () => {
    expect(extensionForLanguage("TypeScript")).toBe("ts");
    expect(extensionForLanguage("c++")).toBe("cpp");
    expect(extensionForLanguage("bash")).toBe("sh");
    expect(extensionForLanguage("")).toBe("txt");
    expect(extensionForLanguage("brainfuck")).toBe("txt");
  });
});

describe("CodeBlock save", () => {
  it("writes the fence to the path the native picker returns", async () => {
    render(<CodeBlock language="ts" code={"const a = 1;\n"} />);
    fireEvent.click(screen.getByRole("button", { name: "ファイルに保存" }));

    await waitFor(() =>
      expect(saveTextFile).toHaveBeenCalledWith(
        "/tmp/snippet.ts",
        "const a = 1;\n",
      ),
    );
    expect(pickSaveFile).toHaveBeenCalledWith(
      expect.objectContaining({ defaultName: "snippet.ts" }),
    );
    expect(useToasts.getState().toasts[0]?.message).toBe("保存しました");
  });

  it("does nothing when the picker is cancelled", async () => {
    pickSaveFile.mockResolvedValueOnce(null);
    render(<CodeBlock language="js" code="console.log(1);" />);
    fireEvent.click(screen.getByRole("button", { name: "ファイルに保存" }));

    await waitFor(() => expect(pickSaveFile).toHaveBeenCalled());
    expect(saveTextFile).not.toHaveBeenCalled();
  });
});
