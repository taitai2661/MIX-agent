import { t } from "@/app/i18n";
import { CornerDownLeft, ShieldCheck, Sparkles } from "lucide-react";

import {
  greetingText,
  suggestionLabelId,
  suggestionPromptId,
  welcomeNote,
  welcomeTitle,
  type Welcome,
} from "./greeting";

export type ChatWelcomeProps = {
  loading: boolean;
  welcome: Welcome | null;
  onPickSuggestion: (prompt: string) => void;
};

/** New-conversation hero, or the transcript skeleton while history loads. */
export function ChatWelcome({
  loading,
  welcome,
  onPickSuggestion,
}: ChatWelcomeProps) {
  if (loading) {
    return (
      <div
        className="messages message-skeleton-list"
        aria-busy="true"
        aria-label={t("読み込み中…")}
      >
        {[0, 1, 2].map((i) => (
          <div className="message-skeleton" key={i}>
            <div className="skeleton skeleton-title" />
            <div className="skeleton skeleton-text" />
            <div className="skeleton skeleton-text short" />
          </div>
        ))}
      </div>
    );
  }
  if (!welcome) return null;
  return (
    <section className="welcome">
      <div className="welcome-icon">
        <Sparkles size={30} />
      </div>
      <p className="eyebrow">MIX</p>
      <h1>{welcomeTitle(welcome)}</h1>
      <div className="welcome-note">
        <ShieldCheck size={14} /> {welcomeNote(welcome)}
      </div>
      <div className="suggestions">
        {welcome.suggestions.map((suggestion) => (
          <button
            key={suggestion.id}
            onClick={() =>
              onPickSuggestion(
                greetingText(
                  suggestionPromptId(suggestion.id),
                  suggestion.prompt,
                ),
              )
            }
          >
            <b>
              {greetingText(suggestionLabelId(suggestion.id), suggestion.label)}
            </b>
            <CornerDownLeft size={15} />
          </button>
        ))}
      </div>
    </section>
  );
}
