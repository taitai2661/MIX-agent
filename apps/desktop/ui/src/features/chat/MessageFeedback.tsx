import { useEffect, useRef, useState } from "react";
import { ThumbsDown, ThumbsUp } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import type { MessageFeedbackValue } from "@/lib/types";
import { Button } from "@/components/ui/button";

/**
 * 👍 / 👎 for an Auto-routed answer.
 *
 * Only rendered when the message carries an `autoSelection` (the backend
 * rejects feedback on manually-routed answers). Clicking the active button
 * again clears the rating — `set_message_feedback` accepts `null` for that.
 *
 * The transcript query is not invalidated: the rating feeds future Auto
 * routing, not the rendered content, so a local optimistic value is enough and
 * avoids re-fetching the whole page on every click.
 */
export function MessageFeedback({
  messageId,
  initial,
}: {
  messageId: string;
  initial: MessageFeedbackValue | null;
}) {
  const [value, setValue] = useState<MessageFeedbackValue | null>(initial);
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  // Remember the last server value so a transcript refetch (which can carry a
  // rating changed elsewhere) is adopted, while an in-flight optimistic click
  // is not overwritten.
  const syncedInitial = useRef<MessageFeedbackValue | null>(initial);
  useEffect(() => {
    if (initial === syncedInitial.current) return;
    // Record the new server value even while a click is in flight. Skipping
    // the ref meant the optimistic value was later "restored" to a stale one
    // when the request finished.
    syncedInitial.current = initial;
    if (!pending) setValue(initial);
  }, [initial, pending]);

  const rate = async (next: MessageFeedbackValue) => {
    if (pending) return;
    const target = value === next ? null : next;
    const previous = value;
    setValue(target);
    setPending(true);
    setFailed(false);
    try {
      await api.setMessageFeedback(messageId, target);
    } catch {
      setValue(previous);
      setFailed(true);
    } finally {
      setPending(false);
    }
  };

  return (
    <>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-disabled={pending}
        disabled={pending}
        onClick={() => void rate("up")}
        aria-pressed={value === "up"}
        aria-label={t("回答を高く評価")}
        title={failed ? t("評価を保存できませんでした") : t("回答を高く評価")}
        className={cn(value === "up" && "text-success")}
      >
        <ThumbsUp size={13} aria-hidden="true" />
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-disabled={pending}
        disabled={pending}
        onClick={() => void rate("down")}
        aria-pressed={value === "down"}
        aria-label={t("回答を低く評価")}
        title={failed ? t("評価を保存できませんでした") : t("回答を低く評価")}
        className={cn(value === "down" && "text-danger")}
      >
        <ThumbsDown size={13} aria-hidden="true" />
      </Button>
      {failed && (
        <span role="alert" className="self-center text-[10px] text-danger">
          {t("評価を保存できませんでした")}
        </span>
      )}
    </>
  );
}
