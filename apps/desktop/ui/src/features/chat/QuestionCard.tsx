/// Structured question card for a run asking the user to choose.
///
/// Unlike an approval (Allow / Deny on a Tool call), this carries an arbitrary
/// question with a fixed set of options. The answer is sent back through the
/// same steering queue as a message typed mid-Run (`answer_question`), so
/// replying is equivalent to steering — it never bypasses the turn boundary.
///
/// Accessibility: the options are native radio/checkbox inputs in a `group`,
/// and the card announces itself through an `aria-live` region without taking
/// focus (same contract as `ApprovalCard`).

import { memo, useState } from "react";
import { MessageCircleQuestion } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import type { RunTimeline } from "@/lib/runEvents";
import { recordUxEvent } from "@/lib/uxEvents";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ErrorBadge } from "@/components/shared";

export const QuestionCard = memo(function QuestionCard({
  live,
}: {
  live: RunTimeline;
}) {
  const card = live.question;
  const [selected, setSelected] = useState<string[]>([]);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!card) return null;
  const multi = card.multiSelect;

  const toggle = (label: string) => {
    setSelected((prev) =>
      multi
        ? prev.includes(label)
          ? prev.filter((item) => item !== label)
          : [...prev, label]
        : [label],
    );
  };

  const canSubmit = !busy && (selected.length > 0 || note.trim() !== "");
  const answer = async () => {
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      const accepted = await api.answerQuestion({
        runId: live.runId,
        questionId: card.questionId,
        selected,
        freeText: note.trim() === "" ? null : note.trim(),
      });
      recordUxEvent("question_answered", {
        location: "card",
        multi_select: multi,
        option_count: selected.length,
        has_note: note.trim() !== "",
      });
      if (!accepted) {
        // The Run finished while the user was choosing. The backend already
        // retired the card, so keep the choice on screen and say where to
        // resend it instead of silently dropping the answer.
        setError(
          t(
            "実行が終了したため回答は届きませんでした。チャット欄から送信し直してください。",
          ),
        );
      }
    } catch (err) {
      // Same contract as the approval card: a failed answer must surface,
      // not silently re-enable the card while the run waits.
      setError(errorMessage(err) || t("エラー"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      role="group"
      aria-label={`${t("質問があります")}: ${card.question}`}
      className="rounded-lg border border-brand/25 bg-brand-soft outline-none"
    >
      <span role="status" className="sr-only">
        {`${t("質問があります")}: ${card.question}`}
      </span>
      <div className="flex flex-wrap items-center gap-1.5 px-3 py-2 text-sm font-medium text-brand">
        <MessageCircleQuestion
          size={15}
          className="shrink-0"
          aria-hidden="true"
        />
        {t("質問があります")}
        {multi && (
          <Badge size="sm" tone="brand">
            {t("複数選択可")}
          </Badge>
        )}
      </div>
      <p className="px-3 pb-1 text-sm text-fg">{card.question}</p>
      <div className="flex flex-col gap-1.5 px-3 py-1">
        {card.options.map((option) => {
          const checked = selected.includes(option.label);
          const recommended = card.recommended === option.label;
          return (
            <label
              key={option.label}
              className="flex cursor-pointer items-start gap-2 rounded-md border border-line bg-surface px-2.5 py-1.5 text-sm transition-colors hover:bg-hover has-checked:border-brand/50"
            >
              <input
                type={multi ? "checkbox" : "radio"}
                name={`question-${card.questionId ?? live.runId}`}
                className="mt-0.5 shrink-0 accent-brand"
                checked={checked}
                disabled={busy}
                onChange={() => toggle(option.label)}
              />
              <span className="min-w-0 flex-1">
                <span className="flex flex-wrap items-center gap-1.5 font-medium text-fg">
                  {option.label}
                  {recommended && (
                    <Badge size="sm" tone="success">
                      {t("おすすめ")}
                    </Badge>
                  )}
                </span>
                {option.description !== "" && (
                  <span className="block text-xs text-muted">
                    {option.description}
                  </span>
                )}
              </span>
            </label>
          );
        })}
      </div>
      <div className="px-3 py-1">
        <label
          htmlFor={`question-note-${card.questionId ?? live.runId}`}
          className="mb-1 block text-xs text-muted"
        >
          {t("補足（任意）")}
        </label>
        <input
          id={`question-note-${card.questionId ?? live.runId}`}
          type="text"
          value={note}
          disabled={busy}
          onChange={(e) => setNote(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && canSubmit) void answer();
          }}
          placeholder={t("選択肢にない希望があれば書いてください")}
          className="w-full rounded-md border border-line bg-surface px-2 py-1.5 text-sm text-fg outline-none placeholder:text-faint focus:border-brand/60"
        />
      </div>
      <div className="flex items-center justify-end gap-2 px-3 py-2">
        {error && (
          <span role="status" className="mr-auto">
            <ErrorBadge error={error} />
          </span>
        )}
        <Button
          variant="primary"
          size="sm"
          disabled={!canSubmit}
          onClick={() => void answer()}
        >
          {t("回答する")}
        </Button>
      </div>
    </div>
  );
});
