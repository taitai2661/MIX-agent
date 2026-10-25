import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, Bot, CornerDownLeft, ShieldCheck, Sparkles } from "lucide-react";
import { t } from "@/lib/i18n";
import { type RunTimeline } from "@/lib/runEvents";
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
import type { MessageView, ModelView, RunView } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { ErrorBox, ListSkeleton } from "@/components/shared";
import { useUi } from "@/lib/ui";

export function Transcript({
  messages,
  runs,
  live,
  liveRunId,
  loading = false,
  error = null,
  welcome = null,
  onEditMessage,
  onRetry,
  editDisabled = false,
  retryDisabled = false,
  models = [],
}: {
  messages: MessageView[];
  runs: Map<string, RunView>;
  live: RunTimeline | null;
  liveRunId: string | null;
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
  /** A Run is in flight: editing would race the transcript the engine appends. */
  editDisabled?: boolean;
  retryDisabled?: boolean;
  /** Registered models, used to resolve a stored `<uuid>:<model>` id to its
   * label. Without this the transcript rendered the raw id. */
  models?: ModelView[];
}) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [stuck, setStuck] = useState(true);
  const prevCountRef = useRef(messages.length);
  const rafRef = useRef(0);
  const stickRafRef = useRef(0);
  const contentKey = `${messages.length}:${live?.text.length ?? 0}:${live?.tools.length ?? 0}`;

  // Scroll events fire at high frequency; coalesce them into one state write
  // per frame so streaming tokens don't re-render the whole list per tick.
  const onScroll = () => {
    if (rafRef.current) return;
    rafRef.current = window.requestAnimationFrame(() => {
      rafRef.current = 0;
      const el = scrollRef.current;
      if (!el) return;
      setStuck(el.scrollHeight - el.scrollTop - el.clientHeight < 48);
    });
  };

  useEffect(
    () => () => {
      if (rafRef.current) window.cancelAnimationFrame(rafRef.current);
      if (stickRafRef.current) window.cancelAnimationFrame(stickRafRef.current);
    },
    [],
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
  }, [contentKey, stuck]);

  const livePersisted =
    !!liveRunId &&
    messages.some((m) => m.run_id === liveRunId && m.role === "assistant");
  const showLive = !!live && !livePersisted;

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

  const isEmpty = messages.length === 0 && !showLive;

  return (
    <div className="relative min-h-0 flex-1">
      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="stable-scroll h-full overflow-y-auto overscroll-contain"
      >
        <div
          className="mx-auto flex w-full max-w-content flex-col gap-6 px-4 py-8 sm:px-6"
          role="log"
          aria-label={t("会話")}
          // Pause the live region while tokens stream: announcing every flush
          // (~16/s) floods screen readers. The completion status below
          // announces once the answer lands instead.
          aria-live={showLive ? "off" : "polite"}
          aria-relevant="additions"
        >
          {error != null && <ErrorBox error={error} />}
          {isEmpty && !error && (
            <Welcome welcome={welcome} hasModels={models.length > 0} />
          )}
          {!isEmpty && loading && messages.length === 0 && !showLive && (
            <ListSkeleton rows={1} />
          )}
          {messages.map((message) => (
            <MessageItem
              key={message.id}
              message={message}
              runs={runs}
              models={models}
              onEdit={onEditMessage}
              onRetry={onRetry}
              editDisabled={editDisabled}
              retryDisabled={retryDisabled || editDisabled}
              showRunError={
                message.role === "assistant"
                  ? lastAssistantByRun.get(message.run_id ?? "") === message.id
                  : orphanRunIds.has(message.run_id ?? "")
              }
            />
          ))}
          {showLive && live && <LiveBlock live={live} />}
          {!showLive && (
            <span className="sr-only" role="status" aria-live="polite">
              {messages.length > 0 ? t("応答が完了しました") : ""}
            </span>
          )}
        </div>
      </div>
      {!stuck && (
        <Button
          variant="default"
          size="sm"
          onClick={() => {
            const el = scrollRef.current;
            if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
            setStuck(true);
          }}
          className="absolute bottom-4 left-1/2 -translate-x-1/2 border-line-strong shadow-pop"
        >
          <ArrowDown size={13} aria-hidden="true" /> {t("最新へ")}
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
    <div className="relative flex flex-col items-center gap-6 py-16 text-center animate-rise">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute -top-6 left-1/2 size-64 -translate-x-1/2 rounded-full bg-brand/10 blur-3xl"
      />
      <div className="relative grid size-14 place-items-center rounded-2xl border border-line bg-gradient-to-b from-surface to-inset text-brand">
        {isAgent ? <Bot size={24} /> : <Sparkles size={24} />}
      </div>
      <div className="flex flex-col gap-2">
        <p className="text-2xs font-bold tracking-[0.18em] text-accent uppercase">
          {isAgent ? t("デスクトップエージェント") : "MIX-agent"}
        </p>
        {welcome ? (
          <h2 className="mx-auto max-w-lg text-2xl font-semibold tracking-tight text-balance text-fg">
            {welcomeTitle(welcome)}
          </h2>
        ) : (
          <h2 className="text-2xl font-semibold tracking-tight text-fg">
            MIX-agent
          </h2>
        )}
        {welcome && (
          <p className="mx-auto flex max-w-md items-center justify-center gap-1.5 text-xs text-faint">
            <ShieldCheck size={13} aria-hidden="true" />
            {welcomeNote(welcome)}
          </p>
        )}
      </div>
      {!hasModels ? (
        <div className="mx-auto flex w-full max-w-md flex-col items-center gap-3 rounded-xl border border-warning/30 bg-warning-soft p-4 text-center">
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
            t("何でも質問してください。/ でコマンド、⌘K で検索。"),
            ["/", "⌘K"],
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
    <div className="grid w-full max-w-xl grid-cols-1 gap-2 sm:grid-cols-2">
      {suggestions.map((suggestion) => (
        <Button
          key={suggestion.id}
          variant="default"
          onClick={() => {
            const detail = promptDetail(suggestion);
            insertPrompt(detail.text, detail.mode);
          }}
          className="group h-auto items-start justify-start gap-2.5 rounded-xl border-line px-3.5 py-3 text-start transition-colors hover:border-line-strong"
        >
          <span className="mt-0.5 grid size-6 shrink-0 place-items-center rounded-md bg-inset text-muted transition-colors group-hover:bg-surface group-hover:text-fg">
            <CornerDownLeft size={12} aria-hidden="true" />
          </span>
          <span className="min-w-0 flex-1 text-xs leading-5 text-fg">
            {greetingText(suggestionLabelId(suggestion.id), suggestion.label)}
          </span>
        </Button>
      ))}
    </div>
  );
}
import { MessageItem, hintWithKeys } from "./MessageItem";
