/// Speech-recognition voice input for the composer.
///
/// Progressive enhancement: without a `SpeechRecognition` constructor the
/// toggle stays in the toolbar (stable layout) but disabled, with the reason
/// in the title instead of failing silently on click.

import { useEffect, useRef, useState } from "react";
import { getLanguage } from "@/lib/i18n";

type Recognition = {
  lang: string;
  continuous: boolean;
  interimResults: boolean;
  onresult:
    | ((event: {
        resultIndex: number;
        results: ArrayLike<ArrayLike<{ transcript: string; isFinal: boolean }>>;
      }) => void)
    | null;
  onend: (() => void) | null;
  onerror: (() => void) | null;
  start: () => void;
  stop: () => void;
  abort: () => void;
};

type RecognitionWindow = Window & {
  SpeechRecognition?: new () => Recognition;
  webkitSpeechRecognition?: new () => Recognition;
};

export function useVoiceInput(
  draft: string,
  setDraft: (value: string) => void,
): {
  listening: boolean;
  toggleVoiceInput: () => void;
  speechSupported: boolean;
} {
  const [listening, setListening] = useState(false);
  const speechRef = useRef<{ stop: () => void; abort: () => void } | null>(
    null,
  );
  // Latest draft seen by the hook; the dictation handler replaces only the
  // segment it inserted, so text typed mid-recognition survives.
  const draftRef = useRef(draft);
  draftRef.current = draft;

  useEffect(() => () => speechRef.current?.abort(), []);

  const speechSupported =
    typeof window !== "undefined" &&
    ("SpeechRecognition" in window || "webkitSpeechRecognition" in window);

  const toggleVoiceInput = () => {
    if (speechRef.current) {
      speechRef.current.stop();
      speechRef.current = null;
      setListening(false);
      return;
    }

    const RecognitionCtor =
      (window as RecognitionWindow).SpeechRecognition ??
      (window as RecognitionWindow).webkitSpeechRecognition;
    if (!RecognitionCtor) return;

    const recognition = new RecognitionCtor();
    const base = draft.trimEnd();
    // Dictation only owns the segment it appended. Rebuilding the whole draft
    // from the snapshot taken at start() would silently delete anything the
    // user typed while the (now continuous) recognition was running. Track the
    // latest draft from the ref so the replacement is computed deterministically
    // (a side-effecting functional setState updater double-fires in StrictMode).
    let lastSuffix = "";
    let spoken = "";
    // The Web Speech API may re-deliver an already-final result in a later
    // event; remember which indices we consumed so they are not appended twice.
    const consumedFinals = new Set<number>();
    recognition.lang = getLanguage() === "ja" ? "ja-JP" : "en-US";
    recognition.continuous = true;
    recognition.interimResults = true;
    recognition.onresult = (event) => {
      let interim = "";
      for (let i = event.resultIndex; i < event.results.length; i += 1) {
        const result = event.results[i];
        if (result[0].isFinal) {
          if (consumedFinals.has(i)) continue;
          consumedFinals.add(i);
          spoken += result[0].transcript;
        } else interim += result[0].transcript;
      }
      const dictation = [spoken, interim].filter(Boolean).join(" ");
      const suffix = dictation ? `${base ? " " : ""}${dictation}` : "";
      const prev = draftRef.current;
      const at = lastSuffix ? prev.lastIndexOf(lastSuffix) : -1;
      let head =
        at >= 0 ? prev.slice(0, at) + prev.slice(at + lastSuffix.length) : prev;
      if (suffix.startsWith(" ")) head = head.trimEnd();
      const next = head + suffix;
      lastSuffix = suffix;
      draftRef.current = next;
      setDraft(next);
    };
    recognition.onend = () => {
      speechRef.current = null;
      setListening(false);
    };
    recognition.onerror = () => {
      // An error does not guarantee the microphone was released; abort so the
      // capture indicator and recognizer are cleaned up before retrying.
      recognition.abort();
      speechRef.current = null;
      setListening(false);
    };
    speechRef.current = recognition;
    setListening(true);
    recognition.start();
  };

  return { listening, toggleVoiceInput, speechSupported };
}
