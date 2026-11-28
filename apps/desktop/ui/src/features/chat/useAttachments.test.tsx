import { act, renderHook } from "@testing-library/react";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { setLanguage } from "@/lib/i18n";
import { useAttachments } from "./useAttachments";

vi.mock("@/lib/api", () => ({
  api: {
    pickOpenFile: vi.fn(async () => []),
    readAttachment: vi.fn(),
  },
}));

beforeAll(() => setLanguage("ja"));

describe("useAttachments cap", () => {
  it("reports files dropped by the attachment cap instead of silently ignoring them", async () => {
    const { result } = renderHook(() => useAttachments());
    const files = Array.from(
      { length: 9 },
      (_, i) => new File(["x"], `f${i}.png`, { type: "image/png" }),
    );

    await act(async () => {
      await result.current.addImageFiles(files);
    });

    expect(result.current.attachments).toHaveLength(8);
    expect(result.current.attachError).toContain("添付は最大8件までです");
  });
});
