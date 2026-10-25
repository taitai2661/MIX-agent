/// Speech-recognition voice input for the composer.
///
/// Progressive enhancement: without a `SpeechRecognition` constructor the
/// toggle stays in the toolbar (stable layout) but disabled, with the reason
/// in the title instead of failing silently on click.

import { useEffect, useRef, useState } from "react";
import { getLanguage } from "@/lib/i18n";

type Recognition = {
  lang: string;
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
    const initialDraft = draft.trimEnd();
    let spoken = "";
    recognition.lang = getLanguage() === "ja" ? "ja-JP" : "en-US";
    recognition.interimResults = true;
    recognition.onresult = (event) => {
      let interim = "";
      for (let i = event.resultIndex; i < event.results.length; i += 1) {
        const result = event.results[i];
        if (result[0].isFinal) spoken += result[0].transcript;
        else interim += result[0].transcript;
      }
      setDraft(
        [initialDraft, spoken, interim]
          .filter(Boolean)
          .join(initialDraft ? " " : ""),
      );
    };
    recognition.onend = () => {
      speechRef.current = null;
      setListening(false);
    };
    recognition.onerror = () => {
      speechRef.current = null;
      setListening(false);
    };
    speechRef.current = recognition;
    setListening(true);
    recognition.start();
  };

  return { listening, toggleVoiceInput, speechSupported };
}
