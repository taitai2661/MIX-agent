import {
  Download,
  MoreHorizontal,
  Pencil,
  Pin,
  PinOff,
  Trash2,
} from "lucide-react";
import { t, tf } from "@/lib/i18n";
import { absoluteTime, relativeTime } from "@/lib/format";
import type { ConversationSummary } from "@/lib/types";
import { cn } from "@/lib/cn";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SectionLabel } from "@/components/layout";
import { SearchEmptyState } from "@/components/common";
import { EmptyState, ErrorBox, ListSkeleton } from "@/components/shared";
import { Spinner } from "@/components/ui/spinner";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  GROUP_LABEL,
  GROUP_ORDER,
  MODE_LABEL,
  type ExtendedThreadGroup,
} from "./threadGroups";

export interface ThreadListProps {
  groups: Map<ExtendedThreadGroup, ConversationSummary[]>;
  selectedId: string | null;
  pinned: Set<string>;
  running: Set<string>;
  renamingId: string | null;
  renameDraft: string;
  onRenameDraftChange: (value: string) => void;
  onCancelRename: () => void;
  onStartRename: (thread: ConversationSummary) => void;
  onCommitRename: (id: string, title: string) => void;
  onOpen: (thread: ConversationSummary) => void;
  onTogglePin: (id: string) => void;
  onExport: (thread: ConversationSummary) => void;
  onRemove: (thread: ConversationSummary) => void;
  removePending: boolean;
  now: number;
  isAgent: boolean;
  searching: boolean;
  serverSearchPending: boolean;
  filteredCount: number;
  showThreadList: boolean;
  hasMore: boolean;
  loadingMore: boolean;
  onLoadMore: () => void;
  onClearSearch: () => void;
  conversationsError: unknown;
  conversationsPending: boolean;
  otherSurfaceLabel: string;
  onSwitchSurface: () => void;
}

export function ThreadList(props: ThreadListProps) {
  const {
    groups,
    selectedId,
    pinned,
    running,
    renamingId,
    renameDraft,
    onRenameDraftChange,
    onCancelRename,
    onStartRename,
    onCommitRename,
    onOpen,
    onTogglePin,
    onExport,
    onRemove,
    removePending,
    now,
    isAgent,
    searching,
    serverSearchPending,
    filteredCount,
    showThreadList,
    hasMore,
    loadingMore,
    onLoadMore,
    onClearSearch,
    conversationsError,
    conversationsPending,
    otherSurfaceLabel,
    onSwitchSurface,
  } = props;

  return (
    // eslint-disable-next-line jsx-a11y/no-static-element-interactions -- key delegation over native buttons
    <div
      onKeyDown={(event) => {
        if (
          event.key !== "ArrowDown" &&
          event.key !== "ArrowUp" &&
          event.key !== "Home" &&
          event.key !== "End"
        ) {
          return;
        }
        const target = event.target as HTMLElement;
        if (!target.matches("[data-thread-open]")) return;
        const buttons = Array.from(
          event.currentTarget.querySelectorAll<HTMLButtonElement>(
            "[data-thread-open]",
          ),
        );
        const index = buttons.indexOf(target as HTMLButtonElement);
        if (index === -1) return;
        event.preventDefault();
        const last = buttons.length - 1;
        const next =
          event.key === "ArrowDown"
            ? Math.min(index + 1, last)
            : event.key === "ArrowUp"
              ? Math.max(index - 1, 0)
              : event.key === "Home"
                ? 0
                : last;
        buttons[next]?.focus();
      }}
      onScroll={(event) => {
        const el = event.currentTarget;
        if (el.scrollHeight - el.scrollTop - el.clientHeight < 160) {
          onLoadMore();
        }
      }}
      className="stable-scroll flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-3 pt-1.5"
    >
      {showThreadList ? (
        <>
          {GROUP_ORDER.map((group) => {
            const items = groups.get(group);
            if (!items?.length) return null;
            return (
              <section key={group}>
                <SectionLabel
                  trailing={
                    <span className="font-mono font-normal text-2xs text-faint">
                      {items.length}
                    </span>
                  }
                >
                  {t(GROUP_LABEL[group])}
                </SectionLabel>
                <ul className="flex flex-col gap-0.5">
                  {items.map((thread) => (
                    <li
                      key={thread.id}
                      className="[content-visibility:auto] [contain-intrinsic-size:auto_64px]"
                    >
                      <div
                        className={cn(
                          "group relative flex flex-col gap-1 rounded-lg py-2 pe-9 ps-2.5",
                          "transition-colors duration-100 hover:bg-hover",
                          selectedId === thread.id &&
                            "bg-brand-soft ring-1 ring-brand/20 ring-inset",
                        )}
                      >
                        {renamingId === thread.id ? (
                          <Input
                            className="h-7 px-2 text-sm"
                            value={renameDraft}
                            // Mounted only while renaming: focus must enter the
                            // field or keyboard users are stranded.
                            // eslint-disable-next-line jsx-a11y/no-autofocus
                            autoFocus
                            onChange={(e) =>
                              onRenameDraftChange(e.target.value)
                            }
                            onKeyDown={(e) => {
                              e.stopPropagation();
                              if (e.key === "Enter" && renameDraft.trim()) {
                                // Commit through blur so Enter + blur cannot
                                // fire the rename twice.
                                e.currentTarget.blur();
                              }
                              if (e.key === "Escape") onCancelRename();
                            }}
                            onBlur={() =>
                              onCommitRename(thread.id, thread.title)
                            }
                          />
                        ) : (
                          <>
                            <button
                              type="button"
                              data-thread-open
                              onClick={() => onOpen(thread)}
                              aria-current={
                                selectedId === thread.id ? "true" : undefined
                              }
                              className="flex min-w-0 items-center gap-1.5 text-start after:absolute after:inset-0 after:z-0 after:content-['']"
                            >
                              {thread.project_id && (
                                <span
                                  className="size-1.5 shrink-0 rounded-full bg-brand"
                                  aria-hidden="true"
                                  title={t("プロジェクト紐付けあり")}
                                />
                              )}
                              {running.has(thread.id) && (
                                <span
                                  role="img"
                                  aria-label={t("実行中")}
                                  title={t("実行中")}
                                  className="size-1.5 shrink-0 animate-pulse rounded-full bg-success"
                                />
                              )}
                              <span
                                className="truncate text-sm font-medium text-fg"
                                title={thread.title}
                              >
                                {thread.title}
                              </span>
                              {pinned.has(thread.id) && (
                                <Pin
                                  size={12}
                                  className="shrink-0 text-brand fill-brand/30"
                                  aria-hidden="true"
                                />
                              )}
                            </button>
                            {thread.preview && (
                              <span className="pointer-events-none truncate text-2xs text-muted">
                                {thread.preview}
                              </span>
                            )}
                            <span className="pointer-events-none flex items-center gap-1 text-2xs text-faint">
                              <time
                                dateTime={thread.updated_at}
                                title={absoluteTime(thread.updated_at)}
                              >
                                {relativeTime(thread.updated_at, new Date(now))}
                              </time>
                              <span className="text-line">·</span>
                              <span>
                                {t(MODE_LABEL[thread.mode] ?? thread.mode)}
                              </span>
                            </span>

                            <DropdownMenu>
                              <DropdownMenuTrigger asChild>
                                <Button
                                  variant="ghost"
                                  size="icon-sm"
                                  title={t("スレッド操作")}
                                  aria-label={t("スレッド操作")}
                                  className={cn(
                                    "absolute end-1 top-1.5 z-10 bg-sidebar/90 shadow-xs backdrop-blur-sm",
                                    "opacity-0 transition-opacity duration-100",
                                    "group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100",
                                    selectedId === thread.id && "opacity-100",
                                  )}
                                >
                                  <MoreHorizontal size={14} />
                                </Button>
                              </DropdownMenuTrigger>
                              <DropdownMenuContent
                                align="end"
                                className="min-w-44"
                              >
                                <DropdownMenuItem
                                  onSelect={() => onTogglePin(thread.id)}
                                >
                                  {pinned.has(thread.id) ? (
                                    <PinOff size={13} className="text-muted" />
                                  ) : (
                                    <Pin size={13} className="text-muted" />
                                  )}
                                  <span className="flex-1">
                                    {pinned.has(thread.id)
                                      ? t("ピン留めを解除")
                                      : t("ピン留め")}
                                  </span>
                                </DropdownMenuItem>
                                <DropdownMenuItem
                                  onSelect={() => onStartRename(thread)}
                                >
                                  <Pencil size={13} className="text-muted" />
                                  <span className="flex-1">
                                    {t("名前を変更")}
                                  </span>
                                </DropdownMenuItem>
                                <DropdownMenuItem
                                  onSelect={() => onExport(thread)}
                                >
                                  <Download size={13} className="text-muted" />
                                  <span className="flex-1">
                                    {t("エクスポート")}
                                  </span>
                                </DropdownMenuItem>
                                <DropdownMenuSeparator />
                                <DropdownMenuItem
                                  disabled={removePending}
                                  onSelect={() => onRemove(thread)}
                                  className="text-danger focus:bg-danger-soft focus:text-danger"
                                >
                                  <Trash2 size={13} />
                                  <span className="flex-1">
                                    {removePending ? t("削除中…") : t("削除")}
                                  </span>
                                </DropdownMenuItem>
                              </DropdownMenuContent>
                            </DropdownMenu>
                          </>
                        )}
                      </div>
                    </li>
                  ))}
                </ul>
              </section>
            );
          })}
          {filteredCount === 0 && searching && serverSearchPending && (
            <ListSkeleton rows={2} label={t("読み込み中…")} />
          )}
          {filteredCount === 0 &&
            (!searching || !serverSearchPending) &&
            (!hasMore || searching) && (
              <SearchEmptyState
                compact
                title={
                  isAgent
                    ? t("一致するコーディングがありません")
                    : t("一致するチャットがありません")
                }
                onClear={onClearSearch}
              />
            )}
          {hasMore && !searching && (
            <div className="flex justify-center pt-1">
              <Button
                size="sm"
                variant="ghost"
                onClick={onLoadMore}
                disabled={loadingMore}
                className="text-muted"
              >
                {loadingMore ? (
                  <Spinner size={11} label={t("読み込み中…")} />
                ) : (
                  t("以前のスレッドを読み込む")
                )}
              </Button>
            </div>
          )}
        </>
      ) : conversationsError ? (
        <ErrorBox error={conversationsError || t("エラー")} />
      ) : conversationsPending ? (
        <ListSkeleton rows={3} label={t("読み込み中…")} />
      ) : (
        <EmptyState
          compact
          title={
            isAgent
              ? t("コーディング履歴がありません")
              : t("チャット履歴がありません")
          }
          sub={
            isAgent
              ? t("新しいコーディングを始めましょう")
              : t("新しい会話を始めましょう")
          }
          action={
            <Button size="sm" variant="ghost" onClick={onSwitchSurface}>
              {tf("{label}の履歴を見る", { label: otherSurfaceLabel })}
            </Button>
          }
        />
      )}
    </div>
  );
}
