import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, ListChecks, Pencil, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { t, tf } from "@/lib/i18n";
import { isBusy } from "@/lib/runEvents";
import { useRunEvents } from "@/lib/runStore";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ErrorBadge } from "@/components/shared";

/**
 * Messages typed while a Run is streaming wait in the engine's in-memory queue
 * and are folded in at the next turn boundary. They used to be invisible until
 * that boundary — a user who sent a correction could not tell whether it was
 * queued, ignored, or already applied. This strip shows the queue and allows
 * editing/removing a message before the model sees it.
 *
 * The queue is drained between turns, so polling stops as soon as the Run is
 * no longer busy and an empty result means "nothing pending", not an error.
 */
export function SteeringQueue({
  conversationId,
}: {
  conversationId: string | null;
}) {
  const runId = useRunEvents((s) =>
    conversationId ? (s.byConversation[conversationId]?.runId ?? null) : null,
  );
  const status = useRunEvents((s) =>
    conversationId
      ? (s.byConversation[conversationId]?.timeline.status ?? null)
      : null,
  );
  const busy = !!status && isBusy(status);
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement | null>(null);

  const queue = useQuery({
    queryKey: ["steering", runId],
    queryFn: () => api.listSteering(runId!),
    enabled: !!runId && busy,
    refetchInterval: 2_000,
    staleTime: 1_000,
  });

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: ["steering", runId] });
  };

  const update = useMutation({
    mutationFn: (input: { seq: number; content: string }) =>
      api.updateSteering(runId!, input.seq, input.content),
    onSuccess: () => {
      setEditing(null);
      invalidate();
    },
  });

  const remove = useMutation({
    mutationFn: (seq: number) => api.removeSteering(runId!, seq),
    onSuccess: () => invalidate(),
  });

  useEffect(() => {
    if (editing === null) return;
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [editing]);

  // A Run that finishes/refreshes can consume the message being edited.
  useEffect(() => {
    if (!busy) setEditing(null);
  }, [busy]);

  const items = queue.data ?? [];
  if (!runId || !busy || items.length === 0) return null;

  const error = update.error ?? remove.error;

  return (
    <div className="shrink-0 border-t border-line bg-surface">
      <div className="mx-auto flex w-full max-w-content flex-col gap-1 px-4 py-2 sm:px-6">
        <div className="flex items-center gap-1.5 text-2xs text-faint">
          <ListChecks size={11} aria-hidden="true" />
          <span>
            {tf("{count}件の指示を次のターンで送信します", {
              count: items.length,
            })}
          </span>
        </div>
        {error !== null && <ErrorBadge error={error} />}
        <ul className="flex flex-col gap-1">
          {items.map((item) => (
            <li
              key={item.seq}
              className="flex min-w-0 items-start gap-1.5 rounded-md border border-line bg-inset/40 px-2 py-1"
            >
              {editing === item.seq ? (
                <>
                  <Input
                    ref={inputRef}
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Escape") {
                        e.preventDefault();
                        setEditing(null);
                      } else if (e.key === "Enter") {
                        e.preventDefault();
                        if (draft.trim() && !update.isPending) {
                          update.mutate({ seq: item.seq, content: draft });
                        }
                      }
                    }}
                    aria-label={t("指示を編集")}
                    className="h-7 min-w-0 flex-1 text-xs"
                  />
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    disabled={!draft.trim() || update.isPending}
                    onClick={() =>
                      update.mutate({ seq: item.seq, content: draft })
                    }
                    title={t("保存")}
                    aria-label={t("保存")}
                  >
                    <Check size={12} />
                  </Button>
                </>
              ) : (
                <>
                  <span className="min-w-0 flex-1 whitespace-pre-wrap break-words text-xs text-fg">
                    {item.content}
                  </span>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    onClick={() => {
                      setDraft(item.content);
                      setEditing(item.seq);
                    }}
                    title={t("指示を編集")}
                    aria-label={t("指示を編集")}
                  >
                    <Pencil size={11} />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    className="hover:text-danger"
                    disabled={remove.isPending}
                    onClick={() => remove.mutate(item.seq)}
                    title={t("指示を削除")}
                    aria-label={t("指示を削除")}
                  >
                    <Trash2 size={11} />
                  </Button>
                </>
              )}
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
