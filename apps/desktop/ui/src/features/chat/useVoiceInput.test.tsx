import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { setLanguage } from "@/lib/i18n";
import { useVoiceInput } from "./useVoiceInput";

type Alternative = { transcript: string; isFinal: boolean };
type ResultEvent = {
  resultIndex: number;
  results: ArrayLike<ArrayLike<Alternative>>;
};

class FakeRecognition {
  static instances: FakeRecognition[] = [];
  lang = "";
  continuous = false;
  interimResults = false;
  onresult: ((event: ResultEvent) => void) | null = null;
  onend: (() => void) | null = null;
  onerror: (() => void) | null = null;
  started = 0;
  stopped = 0;
  aborted = 0;

  constructor() {
    FakeRecognition.instances.push(this);
  }
  start() {
    this.started += 1;
  }
  stop() {
    this.stopped += 1;
  }
  abort() {
    this.aborted += 1;
  }
  emit(resultIndex: number, results: Alternative[][]) {
    this.onresult?.({ resultIndex, results });
  }
}

const recognitionWindow = window as Window & {
  SpeechRecognition?: new () => FakeRecognition;
};
const previousCtor = recognitionWindow.SpeechRecognition;

beforeAll(() => setLanguage("ja"));
afterEach(() => {
  recognitionWindow.SpeechRecognition = previousCtor;
  FakeRecognition.instances = [];
});

describe("useVoiceInput", () => {
  it("sets continuous mode and ignores re-delivered final results", () => {
    recognitionWindow.SpeechRecognition = FakeRecognition;
    const setDraft = vi.fn();
    const { result } = renderHook(() => useVoiceInput("", setDraft));

    act(() => result.current.toggleVoiceInput());
    const recognition = FakeRecognition.instances[0];
    expect(recognition.continuous).toBe(true);
    expect(recognition.lang).toBe("ja-JP");
    expect(recognition.started).toBe(1);

    act(() =>
      recognition.emit(0, [[{ transcript: "こんにちは", isFinal: true }]]),
    );
    expect(setDraft).toHaveBeenLastCalledWith("こんにちは");

    // A later event re-delivers index 0 as final and adds a new phrase.
    act(() =>
      recognition.emit(0, [
        [{ transcript: "こんにちは", isFinal: true }],
        [{ transcript: "世界", isFinal: true }],
      ]),
    );
    expect(setDraft).toHaveBeenLastCalledWith("こんにちは世界");
    expect(setDraft.mock.calls.at(-1)?.[0]).not.toContain(
      "こんにちは こんにちは",
    );
  });

  it("keeps the existing draft and appends an interim result", () => {
    recognitionWindow.SpeechRecognition = FakeRecognition;
    const setDraft = vi.fn();
    const { result } = renderHook(() => useVoiceInput("下書き ", setDraft));
    act(() => result.current.toggleVoiceInput());
    const recognition = FakeRecognition.instances[0];
    act(() => recognition.emit(0, [[{ transcript: "途中", isFinal: false }]]));
    expect(setDraft).toHaveBeenLastCalledWith("下書き 途中");
  });

  it("aborts an active recognition on unmount", () => {
    recognitionWindow.SpeechRecognition = FakeRecognition;
    const { result, unmount } = renderHook(() => useVoiceInput("", vi.fn()));
    act(() => result.current.toggleVoiceInput());
    const recognition = FakeRecognition.instances[0];
    unmount();
    expect(recognition.aborted).toBe(1);
  });

  it("keeps text typed while continuous dictation is running", () => {
    recognitionWindow.SpeechRecognition = FakeRecognition;
    const setDraft = vi.fn();
    const { result, rerender } = renderHook(
      ({ draft }: { draft: string }) => useVoiceInput(draft, setDraft),
      { initialProps: { draft: "" } },
    );
    act(() => result.current.toggleVoiceInput());
    const recognition = FakeRecognition.instances[0];
    act(() =>
      recognition.emit(0, [[{ transcript: "こんにちは", isFinal: true }]]),
    );
    expect(setDraft).toHaveBeenLastCalledWith("こんにちは");

    // The user types while the recognizer is still active. The next result must
    // replace only the dictation segment, not rebuild the whole draft.
    rerender({ draft: "メモ: こんにちは" });
    act(() =>
      recognition.emit(0, [
        [{ transcript: "こんにちは", isFinal: true }],
        [{ transcript: "世界", isFinal: true }],
      ]),
    );
    expect(setDraft).toHaveBeenLastCalledWith("メモ: こんにちは世界");
  });

  it("aborts the recognizer when it reports an error", () => {
    recognitionWindow.SpeechRecognition = FakeRecognition;
    const { result } = renderHook(() => useVoiceInput("", vi.fn()));
    act(() => result.current.toggleVoiceInput());
    const recognition = FakeRecognition.instances[0];
    act(() => recognition.onerror?.());
    expect(recognition.aborted).toBe(1);
    expect(result.current.listening).toBe(false);
  });
});
