/// Visible marker for automatic context compression.
///
/// The engine summarises older turns once the selected model's context window
/// fills up. Without this marker the transcript looked identical before and
/// after the summary was spliced in — so a model that had legitimately
/// forgotten an early instruction appeared to be ignoring it.

import { Archive } from "lucide-react";
import { t, tf } from "@/lib/i18n";
import { cn } from "@/lib/cn";
import type { ContextCompaction } from "@/lib/runEvents";

export function ContextCompactionNotice({
  items,
  className,
}: {
  items: ContextCompaction[];
  className?: string;
}) {
  if (items.length === 0) return null;
  const latest = items[items.length - 1]!;
  return (
    <details
      className={cn(
        "group overflow-hidden rounded-lg border border-line bg-inset/60 text-xs shadow-xs",
        className,
      )}
    >
      <summary className="flex cursor-pointer list-none items-center gap-2 px-2.5 py-1.5 text-muted transition-colors hover:bg-hover hover:text-fg">
        <Archive size={13} className="shrink-0 text-brand" aria-hidden="true" />
        <span className="font-medium">{t("履歴を圧縮しました")}</span>
        {items.length > 1 && (
          <span className="rounded-full bg-surface px-1.5 py-0.5 font-mono text-2xs text-faint">
            {tf("{count} 回", { count: items.length })}
          </span>
        )}
        <span className="ms-auto truncate text-2xs text-faint">
          {tf("直近 {kept} 件を保持", { kept: latest.messages })}
        </span>
      </summary>
      <div className="flex flex-col gap-1.5 border-t border-line px-2.5 py-2 text-2xs text-muted">
        <p className="m-0">
          {tf(
            "古いメッセージ {source} 件を要約し、直近 {kept} 件をそのまま保持しています。",
            { source: latest.sourceMessages, kept: latest.messages },
          )}
        </p>
        {latest.evictedToolResults > 0 && (
          <p className="m-0">
            {tf("ツール結果 {count} 件を要約に退避しました。", {
              count: latest.evictedToolResults,
            })}
          </p>
        )}
        <p className="m-0 font-mono text-faint">
          {tf("要約 {chars} 文字 / アーカイブ {rows} 件", {
            chars: latest.activeChars,
            rows: latest.archiveRows,
          })}
        </p>
        {items.length > 1 && (
          <ul className="m-0 flex list-none flex-col gap-0.5 p-0 pt-1 font-mono text-faint">
            {items.slice(0, -1).map((item, index) => (
              <li key={`${item.at}-${index}`}>
                {new Date(item.at).toLocaleTimeString()} ·{" "}
                {tf("{count} 件 → {kept} 件", {
                  count: item.sourceMessages,
                  kept: item.messages,
                })}
              </li>
            ))}
          </ul>
        )}
      </div>
    </details>
  );
}
