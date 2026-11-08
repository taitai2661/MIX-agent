import {
  Fragment,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  ArrowDown,
  Bot,
  ChevronDown,
  ChevronUp,
  CornerDownLeft,
  Search,
  ShieldCheck,
  Sparkles,
  X,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { t, tf } from "@/lib/i18n";
import { chord } from "@/lib/platform";
import { useNow } from "@/lib/clock";
import { dayKey, dayLabel } from "@/lib/format";
import { prefersReducedMotion } from "@/lib/motion";
import {
  promptDetail,
  welcomeTitle,
  welcomeNote,
  greetingText,
  suggestionLabelId,
  type Suggestion,
  type Welcome,
} from "./greeting";
import { usePromptInsert } from "@/lib/promptInsert";
import { LiveBlock } from "./LiveBlock";
import { useRunEvents } from "@/lib/runStore";
import type { MessageView, ModelView, RunView } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { ErrorBox, ListSkeleton } from "@/components/shared";
import { useUi } from "@/lib/ui";

type TranscriptRow =
  | { kind: "error"; key: string }
  | { kind: "older"; key: string }
  | { kind: "day"; key: string; at: string }
  | { kind: "message"; key: string; message: MessageView; index: number }
  | { kind: "live"; key: string }
  | { kind: "announce"; key: string };

/** Below this many messages the plain DOM list is cheaper than measuring every
 *  row; above it, only the viewport (plus overscan) is mounted. */
const VIRTUALIZE_AFTER = 40;

/** Last reading position per thread. Module-scoped on purpose: the component
 *  is remounted by `key={conversationId}`, so React state died on every switch
 *  and the old code always reopened a thread at the bottom. */
const scrollPositions = new Map<string, { top: number; stuck: boolean }>();

export function Transcript({
  conversationId,
  messages,
  runs,
  liveRunId,
  liveConversationId,
  loading = false,
  error = null,
  welcome = null,
  onEditMessage,
  onRetry,
  onFork,
  onReload,
  editDisabled = false,
  retryDisabled = false,
  models = [],
  hasOlder = false,
  loadingOlder = false,
  onLoadOlder,
}: {
  /** Thread whose reading position is remembered across switches. */
  conversationId?: string | null;
  messages: MessageView[];
  runs: Map<string, RunView>;
  liveRunId: string | null;
  /** Conversation the live run belongs to; `null` while no run is live. */
  liveConversationId: string | null;
  loading?: boolean;
  /** Set when the message history could not be loaded — without it a failed
   * fetch rendered as "no messages yet", i.e. as an empty conversation. */
  error?: unknown;
  /** Empty-state copy. Null while the workspace snapshot is still loading,
   * in which case the brand line renders on its own. */
  welcome?: Welcome | null;
  /** Edit a user turn and regenerate; absent on read-only transcripts. */
  onEditMessage?: (messageId: string, content: string) => Promise<void>;
  /** Regenerate from the last user turn after a failure. Shown in the run-error
   *  alert so a failed run is one tap away from a retry instead of a dead end. */
  onRetry?: () => Promise<void> | void;
  /** Copy the conversation prefix into a new thread from this message. */
  onFork?: (messageId: string) => Promise<void> | void;
  /** Retry the failed history fetch. Without it a load error was a dead end:
   *  the only recovery was switching threads or restarting the app. */
  onReload?: () => void;
  /** A Run is in flight: editing would race the transcript the engine appends. */
  editDisabled?: boolean;
  retryDisabled?: boolean;
  /** Registered models, used to resolve a stored `<uuid>:<model>` id to its
   * label. Without this the transcript rendered the raw id. */
  models?: ModelView[];
  /** True when the initial page was full: older rows exist on the server. */
  hasOlder?: boolean;
  loadingOlder?: boolean;
  onLoadOlder?: () => void;
}) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const findInputRef = useRef<HTMLInputElement | null>(null);
  const [findOpen, setFindOpen] = useState(false);
  const [findQuery, setFindQuery] = useState("");
  const [findMatchIndex, setFindMatchIndex] = useState(0);
  const [flashMessageId, setFlashMessageId] = useState<string | null>(null);
  const now = useNow();
  const savedPosition = conversationId
    ? scrollPositions.get(conversationId)
    : undefined;
  // A thread left scrolled-up reopens where it was; a thread left at the
  // bottom keeps following the live answer.
  const [stuck, setStuck] = useState(savedPosition?.stuck ?? true);
  const restoredRef = useRef(false);
  const prevCountRef = useRef(messages.length);
  const rafRef = useRef(0);
  const stickRafRef = useRef(0);
  // Read inside the store subscription below without making it a dependency
  // (a new listener per scroll state would be wasteful).
  const stuckRef = useRef(stuck);
  stuckRef.current = stuck;

  // Scroll events fire at high frequency; coalesce them into one state write
  // per frame so streaming tokens don't re-render the whole list per tick.
  const onScroll = () => {
    if (rafRef.current) return;
    rafRef.current = window.requestAnimationFrame(() => {
      rafRef.current = 0;
      const el = scrollRef.current;
      if (!el) return;
      const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
      setStuck(atBottom);
      if (conversationId) {
        scrollPositions.set(conversationId, {
          top: el.scrollTop,
          stuck: atBottom,
        });
      }
    });
  };

  // Restore the saved offset once the transcript has rows to scroll. Both the
  // plain and virtual paths use this: the virtualizer also receives it as its
  // `initialOffset`, but it can only act after its container exists.
  useLayoutEffect(() => {
    if (restoredRef.current) return;
    const el = scrollRef.current;
    if (!el || messages.length === 0) return;
    restoredRef.current = true;
    if (savedPosition && !savedPosition.stuck && savedPosition.top > 0) {
      el.scrollTop = savedPosition.top;
    }
  }, [messages.length, savedPosition]);

  useEffect(
    () => () => {
      if (rafRef.current) window.cancelAnimationFrame(rafRef.current);
      if (stickRafRef.current) window.cancelAnimationFrame(stickRafRef.current);
      const el = scrollRef.current;
      if (el && conversationId) {
        scrollPositions.set(conversationId, {
          top: el.scrollTop,
          stuck: stuckRef.current,
        });
      }
    },
    [conversationId],
  );

  // Sending a message while scrolled up left the user staring at old history
  // while the answer streamed off-screen. A newly added user turn always
  // re-sticks the view; manual scroll-up afterwards still works as before.
  useLayoutEffect(() => {
    const prev = prevCountRef.current;
    if (messages.length > prev) {
      const last = messages[messages.length - 1];
      if (last?.role === "user") setStuck(true);
    }
    prevCountRef.current = messages.length;
  }, [messages]);

  // Follow-scroll, batched to one jump per frame: the old `useLayoutEffect`
  // forced a synchronous `scrollTop` write before every paint while tokens
  // streamed (~16×/s), which blocked the frame and visibly juddered. Batching
  // in rAF lets all of a frame's tokens land before a single scroll.
  useEffect(() => {
    if (!stuck) return;
    if (stickRafRef.current) return;
    stickRafRef.current = window.requestAnimationFrame(() => {
      stickRafRef.current = 0;
      const el = scrollRef.current;
      if (el) el.scrollTop = el.scrollHeight;
    });
  }, [stuck, liveRunId, messages.length]);

  // Token updates drive scrolling through the store, not through a prop. The
  // transcript used to re-render on every 60 ms flush just to update
  // `contentKey`; now only the LiveBlock subtree and this one rAF callback see
  // the new text.
  useEffect(() => {
    if (!liveConversationId) return;
    let raf = 0;
    const unsubscribe = useRunEvents.subscribe((state, previous) => {
      const entry = state.byConversation[liveConversationId];
      if (!entry) return;
      const before = previous.byConversation[liveConversationId];
      const text = entry.timeline.text.length;
      const reasoning = entry.timeline.reasoning.length;
      const tools = entry.timeline.tools.length;
      if (
        text === (before?.timeline.text.length ?? -1) &&
        reasoning === (before?.timeline.reasoning.length ?? -1) &&
        tools === (before?.timeline.tools.length ?? -1)
      ) {
        return;
      }
      if (!stuckRef.current || raf) return;
      raf = window.requestAnimationFrame(() => {
        raf = 0;
        const el = scrollRef.current;
        if (el) el.scrollTop = el.scrollHeight;
      });
    });
    return () => {
      unsubscribe();
      if (raf) window.cancelAnimationFrame(raf);
    };
  }, [liveConversationId]);

  const livePersisted =
    !!liveRunId &&
    messages.some((m) => m.run_id === liveRunId && m.role === "assistant");
  const showLive = !!liveRunId && !livePersisted;

  // The last assistant message of each Run: its error is rendered under that
  // one row so a Run with several tool-call turns does not repeat the failure.
  const lastAssistantByRun = useMemo(() => {
    const map = new Map<string, string>();
    for (const m of messages) {
      if (m.role === "assistant" && m.run_id) map.set(m.run_id, m.id);
    }
    return map;
  }, [messages]);

  // A failed Run with no assistant message has nothing to attach its error to
  // on the assistant side, so it is attached to the user turn that started it.
  // Without this a provider error before any text streamed left the transcript
  // completely blank.
  const orphanRunIds = useMemo(() => {
    const linked = new Set(
      messages
        .filter((m) => m.role === "assistant" && m.run_id)
        .map((m) => m.run_id as string),
    );
    return new Set(
      [...runs.values()]
        .filter(
          (run) => !!run.error && !linked.has(run.id) && run.id !== liveRunId,
        )
        .map((run) => run.id),
    );
  }, [runs, messages, liveRunId]);

  // While the history is in flight the transcript is *unknown*, not empty.
  // Rendering the greeting first made every existing thread flash "何でも
  // 質問してください" before its messages arrived — which reads as data loss.
  const isEmpty = !loading && messages.length === 0 && !showLive;
  // The completion announcement used to say "応答が完了しました" even when the
  // user had just pressed Stop. Announce what actually happened.
  const lastAssistantRun = [...messages]
    .reverse()
    .find((m) => m.role === "assistant" && m.run_id)?.run_id;
  const completionLabel =
    lastAssistantRun && runs.get(lastAssistantRun)?.status === "cancelled"
      ? t("停止しました")
      : t("応答が完了しました");

  const virtualize = messages.length > VIRTUALIZE_AFTER;
  const rows = useMemo<TranscriptRow[]>(() => {
    const out: TranscriptRow[] = [];
    // The non-virtual branch renders the error box explicitly before the
    // welcome/loading blocks; only the windowed branch needs it as a row.
    if (error != null && virtualize) out.push({ kind: "error", key: "error" });
    if (hasOlder && onLoadOlder) out.push({ kind: "older", key: "older" });
    messages.forEach((message, index) => {
      const day = dayKey(message.created_at);
      const prevDay =
        index > 0 ? dayKey(messages[index - 1]!.created_at) : null;
      if (day && day !== prevDay) {
        out.push({
          kind: "day",
          key: `day-${day}-${index}`,
          at: message.created_at,
        });
      }
      out.push({ kind: "message", key: message.id, message, index });
    });
    if (showLive) out.push({ kind: "live", key: "live" });
    if (!showLive) out.push({ kind: "announce", key: "announce" });
    return out;
  }, [error, hasOlder, messages, onLoadOlder, showLive, virtualize]);

  // In-thread find (⌘F / Ctrl+F). The transcript is virtualised above 40 rows,
  // so a plain DOM query cannot reach every match; keep the row index and let
  // the virtualizer jump to it.
  const findMatches = useMemo(() => {
    const q = findQuery.trim().toLowerCase();
    if (!q) return [];
    return rows.flatMap((row, rowIndex) =>
      row.kind === "message" &&
      `${row.message.content ?? ""}\n${row.message.reasoning ?? ""}`
        .toLowerCase()
        .includes(q)
        ? [{ rowIndex, message: row.message }]
        : [],
    );
  }, [findQuery, rows]);
  const rowVirtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => {
      const row = rows[index];
      if (!row) return 120;
      if (row.kind === "day") return 48;
      if (row.kind === "older") return 44;
      if (row.kind === "announce") return 1;
      return 260;
    },
    getItemKey: (index) => rows[index]?.key ?? index,
    overscan: 6,
    initialOffset:
      savedPosition && !savedPosition.stuck ? savedPosition.top : undefined,
  });

  // Reset and clamp the highlighted match as the query or result set changes.
  useEffect(() => {
    setFindMatchIndex(0);
  }, [findQuery]);
  useEffect(() => {
    if (findMatchIndex < findMatches.length) return;
    setFindMatchIndex(Math.max(0, findMatches.length - 1));
  }, [findMatchIndex, findMatches.length]);

  // Open find with the platform shortcut, close it with Escape. Skip while the
  // embedded browser owns focus: ⌘F there belongs to the page, not the chat.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;
      if (mod && !event.altKey && event.key.toLowerCase() === "f") {
        const active = document.activeElement as HTMLElement | null;
        if (active?.closest("[data-browser-pane]")) return;
        event.preventDefault();
        setFindOpen(true);
        window.requestAnimationFrame(() => {
          findInputRef.current?.focus();
          findInputRef.current?.select();
        });
        return;
      }
      if (event.key === "Escape" && findOpen) {
        event.preventDefault();
        setFindOpen(false);
        setFindQuery("");
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [findOpen]);

  // Scroll the active hit into view (virtualizer index when the list is
  // windowed, element.scrollIntoView otherwise) and flash its row briefly.
  useEffect(() => {
    if (!findOpen) return;
    const match = findMatches[findMatchIndex];
    if (!match) return;
    if (virtualize) {
      rowVirtualizer.scrollToIndex(match.rowIndex, { align: "center" });
    } else {
      window.requestAnimationFrame(() => {
        const element = scrollRef.current?.querySelector<HTMLElement>(
          `[data-message-id="${match.message.id}"]`,
        );
        // jsdom has no scrollIntoView; the browser does.
        if (typeof element?.scrollIntoView === "function") {
          element.scrollIntoView({ block: "center", behavior: "smooth" });
        }
      });
    }
    setFlashMessageId(match.message.id);
    const timer = window.setTimeout(() => setFlashMessageId(null), 1400);
    return () => window.clearTimeout(timer);
  }, [findMatchIndex, findMatches, findOpen, rowVirtualizer, virtualize]);

  const moveFind = (delta: number) => {
    if (findMatches.length === 0) return;
    setFindMatchIndex(
      (index) => (index + delta + findMatches.length) % findMatches.length,
    );
  };

  const virtualTotalSize = rowVirtualizer.getTotalSize();
  // The virtualizer's total height is 0 on the first paint, so the normal
  // follow-scroll effect lands at `scrollTop = 0`. Once measurements land and
  // the total grows, re-pin to the bottom while the user is still there.
  useEffect(() => {
    if (!virtualize || !stuck) return;
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [virtualize, stuck, virtualTotalSize, messages.length]);

  // Prepending a page of older messages must not throw the user to the top.
  // The row measurement happens after paint, so this is a best-effort anchor:
  // it keeps the reading position when the list is already windowed.
  const firstMessageId = messages[0]?.id ?? null;
  const prevFirstMessageId = useRef(firstMessageId);
  const prevScrollHeight = useRef(0);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (
      prevFirstMessageId.current &&
      firstMessageId !== prevFirstMessageId.current &&
      !stuck
    ) {
      el.scrollTop += el.scrollHeight - prevScrollHeight.current;
    }
    prevFirstMessageId.current = firstMessageId;
    prevScrollHeight.current = el.scrollHeight;
  });

  const renderRow = (row: TranscriptRow) => {
    switch (row.kind) {
      case "error":
        return (
          <ErrorBox
            error={error}
            action={onReload ? t("再読み込み") : undefined}
            onAction={onReload}
          />
        );
      case "older":
        return onLoadOlder ? (
          <div className="flex justify-center">
            <Button
              variant="ghost"
              size="sm"
              onClick={onLoadOlder}
              disabled={loadingOlder}
              className="text-muted"
            >
              {loadingOlder
                ? t("読み込み中…")
                : t("以前のメッセージを読み込む")}
            </Button>
          </div>
        ) : null;
      case "day":
        return (
          <div
            role="separator"
            aria-label={dayLabel(row.at)}
            className="flex items-center gap-3 pt-2 text-2xs text-faint"
          >
            <span className="h-px flex-1 bg-line" aria-hidden="true" />
            <span>{dayLabel(row.at)}</span>
            <span className="h-px flex-1 bg-line" aria-hidden="true" />
          </div>
        );
      case "message":
        return (
          <div
            data-message-id={row.message.id}
            className={cn(
              "rounded-2xl transition-shadow duration-150",
              flashMessageId === row.message.id &&
                "ring-2 ring-brand/50 ring-offset-2 ring-offset-bg",
            )}
          >
            <MessageItem
              message={row.message}
              runs={runs}
              models={models}
              onEdit={onEditMessage}
              onRetry={onRetry}
              onFork={onFork}
              editDisabled={editDisabled}
              retryDisabled={retryDisabled || editDisabled}
              tailCount={messages.length - row.index - 1}
              showRunError={
                row.message.role === "assistant"
                  ? lastAssistantByRun.get(row.message.run_id ?? "") ===
                    row.message.id
                  : orphanRunIds.has(row.message.run_id ?? "")
              }
              now={now}
            />
          </div>
        );
      case "live":
        return liveConversationId ? (
          <LiveEntry conversationId={liveConversationId} />
        ) : null;
      case "announce":
        return (
          <span className="sr-only" role="status" aria-live="polite">
            {messages.length > 0 ? completionLabel : ""}
          </span>
        );
    }
  };

  return (
    <div className="relative min-h-0 flex-1">
      {findOpen && (
        <div
          role="search"
          aria-label={t("スレッド内を検索")}
          className="absolute top-3 right-4 z-30 flex items-center gap-1 rounded-xl border border-line bg-surface/95 p-1.5 shadow-pop backdrop-blur-md"
        >
          <Search
            size={13}
            className="ms-1 shrink-0 text-faint"
            aria-hidden="true"
          />
          <input
            ref={findInputRef}
            value={findQuery}
            onChange={(event) => setFindQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                moveFind(event.shiftKey ? -1 : 1);
              }
            }}
            placeholder={t("スレッド内を検索")}
            aria-label={t("スレッド内を検索")}
            className="h-7 w-44 min-w-0 bg-transparent px-1 text-xs text-fg outline-none placeholder:text-faint"
          />
          <span
            aria-live="polite"
            className="min-w-12 shrink-0 text-center font-mono text-2xs text-faint tabular-nums"
          >
            {findQuery.trim()
              ? `${findMatches.length ? findMatchIndex + 1 : 0}/${findMatches.length}`
              : ""}
          </span>
          <Button
            variant="ghost"
            size="icon-sm"
            disabled={findMatches.length === 0}
            onClick={() => moveFind(-1)}
            title={t("前の一致")}
            aria-label={t("前の一致")}
          >
            <ChevronUp size={13} />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            disabled={findMatches.length === 0}
            onClick={() => moveFind(1)}
            title={t("次の一致")}
            aria-label={t("次の一致")}
          >
            <ChevronDown size={13} />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => {
              setFindOpen(false);
              setFindQuery("");
            }}
            title={t("閉じる")}
            aria-label={t("閉じる")}
          >
            <X size={13} />
          </Button>
        </div>
      )}
      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="stable-scroll h-full overflow-y-auto overscroll-contain"
      >
        <div
          className={cn(
            "mx-auto flex w-full max-w-content flex-col gap-6 px-4 py-8 sm:px-6",
            // The empty state sits in the vertical middle of the pane, like a
            // landing page, instead of hugging the top edge under the titlebar.
            isEmpty && !error && "min-h-full justify-center py-10 sm:py-14",
          )}
          role="log"
          aria-label={t("会話")}
          // Pause the live region while tokens stream: announcing every flush
          // (~16/s) floods screen readers. The completion status below
          // announces once the answer lands instead.
          aria-live={showLive ? "off" : "polite"}
          aria-relevant="additions"
        >
          {virtualize ? (
            <div
              style={{
                height: `${rowVirtualizer.getTotalSize()}px`,
                position: "relative",
                width: "100%",
              }}
            >
              {rowVirtualizer.getVirtualItems().map((item) => (
                <div
                  key={item.key}
                  data-index={item.index}
                  ref={rowVirtualizer.measureElement}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    transform: `translateY(${item.start}px)`,
                  }}
                >
                  <div className="pb-6">{renderRow(rows[item.index]!)}</div>
                </div>
              ))}
            </div>
          ) : (
            <>
              {error != null && (
                <ErrorBox
                  error={error}
                  action={onReload ? t("再読み込み") : undefined}
                  onAction={onReload}
                />
              )}
              {isEmpty && !error && (
                <Welcome welcome={welcome} hasModels={models.length > 0} />
              )}
              {loading && messages.length === 0 && !showLive && (
                <ListSkeleton rows={2} />
              )}
              {rows.map((row) => (
                <Fragment key={row.key}>{renderRow(row)}</Fragment>
              ))}
            </>
          )}
        </div>
      </div>
      {!stuck && (
        <Button
          variant="default"
          size="sm"
          onClick={() => {
            const el = scrollRef.current;
            if (el)
              el.scrollTo({
                top: el.scrollHeight,
                behavior: prefersReducedMotion() ? "auto" : "smooth",
              });
            setStuck(true);
          }}
          className="absolute bottom-4 left-1/2 -translate-x-1/2 rounded-full border-line-strong bg-surface/90 px-3.5 py-1.5 shadow-pop backdrop-blur-md transition-all hover:scale-105 active:scale-95"
          title={t("最新のメッセージへスクロール")}
        >
          {showLive ? (
            <span className="relative flex size-2 shrink-0">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-brand opacity-75" />
              <span className="relative inline-flex size-2 rounded-full bg-brand" />
            </span>
          ) : (
            <ArrowDown size={13} aria-hidden="true" />
          )}
          <span className="text-xs font-medium">
            {showLive ? t("生成中… 最新へ") : t("最新へ")}
          </span>
        </Button>
      )}
    </div>
  );
}

function Welcome({
  welcome,
  hasModels,
}: {
  welcome: Welcome | null;
  hasModels: boolean;
}) {
  const setOnboardingForcedOpen = useUi((s) => s.setOnboardingForcedOpen);
  const surface = useUi((s) => s.surface);
  const isAgent = surface === "desktop-agent";

  return (
    <div className="relative flex flex-col items-center gap-7 text-center animate-rise">
      <div className="grid size-[72px] place-items-center rounded-3xl border border-line bg-surface text-brand">
        {isAgent ? (
          <Bot size={28} strokeWidth={1.6} aria-hidden="true" />
        ) : (
          <Sparkles size={28} strokeWidth={1.6} aria-hidden="true" />
        )}
      </div>
      <div className="flex flex-col gap-3">
        <p className="text-2xs font-bold tracking-[0.22em] text-brand uppercase">
          MIX-AGENT
        </p>
        {welcome ? (
          <h2 className="mx-auto max-w-[38rem] text-[22px] leading-[1.5] font-semibold tracking-tight text-balance text-fg sm:text-[28px] sm:leading-[1.55]">
            {welcomeTitle(welcome)}
          </h2>
        ) : (
          <h2 className="text-[28px] font-semibold tracking-tight text-fg">
            MIX-AGENT
          </h2>
        )}
        {welcome && (
          <p className="mx-auto flex max-w-lg flex-wrap items-center justify-center gap-1.5 text-[13px] text-faint">
            <ShieldCheck size={14} aria-hidden="true" />
            {welcomeNote(welcome)}
          </p>
        )}
      </div>
      {!hasModels ? (
        <div className="mx-auto flex w-full max-w-2xl flex-col items-center gap-3 rounded-2xl border border-warning/30 bg-warning-soft p-5 text-center">
          <div className="flex items-center gap-2 text-sm font-medium text-warning">
            <Sparkles size={16} />
            <span>{t("モデル提供元が接続されていません")}</span>
          </div>
          <p className="text-xs text-muted">
            {t(
              "会話を始めるには、Ollamaなどのローカルモデルを検出するか、APIキーを設定してください。",
            )}
          </p>
          <Button
            variant="primary"
            size="sm"
            onClick={() => setOnboardingForcedOpen(true)}
          >
            {t("初期設定ウィザードを開く")}
          </Button>
        </div>
      ) : welcome ? (
        <WelcomeSuggestions suggestions={welcome.suggestions} />
      ) : (
        <p className="mx-auto max-w-md text-sm text-muted">
          {hintWithKeys(
            tf("何でも質問してください。/ でコマンド、{shortcut} で検索。", {
              shortcut: chord("Mod", "K"),
            }),
            ["/", chord("Mod", "K")],
          )}
        </p>
      )}
    </div>
  );
}

/** Context-driven starters. Each one fills the composer, like the quick
 * prompts it replaces, so nothing is sent without an explicit keystroke. */
function WelcomeSuggestions({
  suggestions,
}: {
  suggestions: readonly Suggestion[];
}) {
  const insertPrompt = usePromptInsert((s) => s.insertPrompt);
  return (
    <div className="grid w-full max-w-2xl grid-cols-1 gap-2.5 sm:grid-cols-2">
      {suggestions.map((suggestion) => (
        <Button
          key={suggestion.id}
          variant="default"
          onClick={() => {
            const detail = promptDetail(suggestion);
            insertPrompt(detail.text, detail.mode);
          }}
          className="group h-auto min-h-[52px] w-full items-center justify-start gap-3 rounded-full px-3.5 py-2 text-start"
        >
          <span className="grid size-8 shrink-0 place-items-center rounded-full bg-inset text-muted transition-colors group-hover:bg-surface group-hover:text-fg">
            <CornerDownLeft size={14} aria-hidden="true" />
          </span>
          <span className="min-w-0 flex-1 pe-1.5 text-sm font-medium leading-5 text-fg">
            {greetingText(suggestionLabelId(suggestion.id), suggestion.label)}
          </span>
        </Button>
      ))}
    </div>
  );
}
import { MessageItem, hintWithKeys } from "./MessageItem";

/** The only component that subscribes to the streaming timeline.
 *
 * Keeping the subscription here (instead of in `Transcript`/`ChatPane`) means
 * a token update re-renders this subtree, not the whole transcript, composer
 * or sidebar.
 */
function LiveEntry({ conversationId }: { conversationId: string }) {
  const entry = useRunEvents((s) => s.byConversation[conversationId]);
  return entry ? <LiveBlock live={entry.timeline} /> : null;
}
