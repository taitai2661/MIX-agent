import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDown, Brain, Check, ChevronRight, CircleAlert, CornerDownLeft, FileText, MessageSquare, ShieldAlert, ShieldCheck, Sparkles, Wrench,
} from "lucide-react";
import { Markdown } from "@/components/Markdown";
import { CopyButton } from "@/components/CopyButton";
import { t, tf } from "@/lib/i18n";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { relativeTime } from "@/lib/format";
import { api } from "@/lib/api";
import { isBusy, describeToolCall, type RunTimeline, type ToolActivity } from "@/lib/runEvents";
import { promptDetail, welcomeTitle, welcomeNote, greetingText, suggestionLabelId, type Suggestion, type Welcome } from "./greeting";
import type { MessageView, RunView } from "@/lib/types";
import { useUi } from "@/lib/ui";
import { Alert, Skeleton } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/layout";
import { Spinner } from "@/components/ui/spinner";

export function Transcript({
  messages,
  runs,
  live,
  liveRunId,
  loading = false,
  error = null,
  welcome = null,
}: {
  messages: MessageView[];
  runs: Map<string, RunView>;
  live: RunTimeline | null;
  liveRunId: string | null;
  loading?: boolean;
  /** Set when the message history could not be loaded — without it a failed
   * fetch rendered as "no messages yet", i.e. as an empty conversation. */
  error?: unknown;
  /** Empty-state copy. Null while the workspace snapshot is still loading, in
   * which case the brand line and the keyboard hints render on their own. */
  welcome?: Welcome | null;
}) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [stuck, setStuck] = useState(true);
  const contentKey = `${messages.length}:${live?.text.length ?? 0}:${live?.tools.length ?? 0}`;

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    setStuck(el.scrollHeight - el.scrollTop - el.clientHeight < 48);
  };

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && stuck) el.scrollTop = el.scrollHeight;
  }, [contentKey, stuck]);

  const livePersisted =
    !!liveRunId && messages.some((m) => m.run_id === liveRunId && m.role === "assistant");
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
        .filter((run) => !!run.error && !linked.has(run.id) && run.id !== liveRunId)
        .map((run) => run.id),
    );
  }, [runs, messages, liveRunId]);

  const isEmpty = messages.length === 0 && !showLive;

  return (
    <div className="relative min-h-0 flex-1">
      <div ref={scrollRef} onScroll={onScroll} className="h-full overflow-y-auto">
        <div
          className="mx-auto flex w-full max-w-content flex-col gap-7 px-6 py-8"
          role="log"
          aria-live="polite"
          aria-relevant="additions text"
          aria-label={t("会話")}
        >
          {error != null && (
            <Alert tone="danger">{errorMessage(error) || t("エラー")}</Alert>
          )}
          {isEmpty && !error && <Welcome welcome={welcome} />}
          {!isEmpty && loading && messages.length === 0 && !showLive && (
            <Skeleton className="h-16 w-full" />
          )}
          {messages.map((message) => (
            <MessageItem
              key={message.id}
              message={message}
              runs={runs}
              showRunError={
                message.role === "assistant"
                  ? lastAssistantByRun.get(message.run_id ?? "") === message.id
                  : orphanRunIds.has(message.run_id ?? "")
              }
            />
          ))}
          {showLive && live && <LiveBlock live={live} />}
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
          className="absolute bottom-4 left-1/2 -translate-x-1/2 border-line-strong"
        >
          <ArrowDown size={13} /> {t("最新へ")}
        </Button>
      )}
    </div>
  );
}

function Welcome({ welcome }: { welcome: Welcome | null }) {
  return (
    <div className="relative flex flex-col items-center gap-6 py-16 text-center animate-rise">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute -top-6 left-1/2 size-64 -translate-x-1/2 rounded-full bg-brand/10 blur-3xl"
      />
      <div className="relative grid size-14 place-items-center rounded-2xl border border-line bg-gradient-to-b from-surface to-inset text-brand">
        <Sparkles size={24} />
      </div>
      <div className="flex flex-col gap-2">
        <p className="text-2xs font-bold tracking-[0.18em] text-accent uppercase">
          MIX-agent
        </p>
        {welcome ? (
          <h2 className="mx-auto max-w-lg text-2xl font-semibold tracking-tight text-balance text-fg">
            {welcomeTitle(welcome)}
          </h2>
        ) : (
          <h2 className="text-2xl font-semibold tracking-tight text-fg">MIX-agent</h2>
        )}
        {welcome && (
          <p className="mx-auto flex max-w-md items-center justify-center gap-1.5 text-xs text-faint">
            <ShieldCheck size={13} aria-hidden="true" />
            {welcomeNote(welcome)}
          </p>
        )}
      </div>
      {welcome ? (
        <WelcomeSuggestions suggestions={welcome.suggestions} />
      ) : (
        <p className="mx-auto max-w-md text-sm text-muted">
          {t("何でも質問してください。/ でコマンド、⌘K で検索。")}
        </p>
      )}
      <div className="flex flex-wrap items-center justify-center gap-x-4 gap-y-1 text-xs text-faint">
        <span className="inline-flex items-center gap-1">
          <Kbd>Enter</Kbd>
          {t("送信")}
        </span>
        <span className="inline-flex items-center gap-1">
          <Kbd>⇧</Kbd>
          +
          <Kbd>Enter</Kbd>
          {t("改行")}
        </span>
        <span className="inline-flex items-center gap-1">
          <Kbd>⌘K</Kbd>
          {t("コマンドパレット")}
        </span>
      </div>
    </div>
  );
}

/** Context-driven starters. Each one fills the composer, like the quick
 * prompts it replaces, so nothing is sent without an explicit keystroke. */
function WelcomeSuggestions({ suggestions }: { suggestions: readonly Suggestion[] }) {
  return (
    <div className="grid w-full max-w-xl grid-cols-1 gap-2 sm:grid-cols-2">
      {suggestions.map((suggestion) => (
        <Button
          key={suggestion.id}
          variant="default"
          onClick={() => {
            window.dispatchEvent(
              new CustomEvent("mix:insert-prompt", { detail: promptDetail(suggestion) }),
            );
          }}
          className="group h-auto items-start justify-start gap-2.5 rounded-xl border-line px-3.5 py-3 text-start transition-colors hover:border-line-strong"
        >
          <span className="mt-0.5 grid size-6 shrink-0 place-items-center rounded-md bg-inset text-muted transition-colors group-hover:bg-surface group-hover:text-fg">
            <CornerDownLeft size={12} aria-hidden="true" />
          </span>
          <span className="min-w-0 flex-1 text-xs leading-5 text-fg">{greetingText(suggestionLabelId(suggestion.id), suggestion.label)}</span>
        </Button>
      ))}
    </div>
  );
}

/** Memoised: the list renders every message on each token of a stream, and a
 * message only depends on its own row plus the run it belongs to. */
const MessageItem = memo(function MessageItem({
  message,
  runs,
  showRunError = false,
}: {
  message: MessageView;
  runs: Map<string, RunView>;
  /** Render this Run's failure under this (its last assistant) message. */
  showRunError?: boolean;
}) {
  const setActivityRunId = useUi((s) => s.setActivityRunId);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const run = message.run_id ? runs.get(message.run_id) : undefined;

  if (message.role === "user") {
    return (
      <div className="flex flex-col items-end gap-1">
        {message.attachments.length > 0 && (
          <div className="flex flex-wrap justify-end gap-1.5">
            {message.attachments.map((a, i) => (
              <span
                key={i}
                className="inline-flex items-center gap-1 rounded-md border border-line bg-inset px-1.5 py-0.5 text-xs text-muted"
              >
                <FileText size={11} />
                {a.filename ?? a.mime_type}
              </span>
            ))}
          </div>
        )}
        <div className="max-w-[85%] whitespace-pre-wrap rounded-2xl rounded-br-md border border-brand/20 bg-brand-soft px-3.5 py-2.5 text-sm leading-6 text-fg">
          {message.content}
        </div>
        <div className="px-1 text-xs text-faint">{relativeTime(message.created_at)}</div>
        {showRunError && run?.error && (
          <Alert tone="danger" className="w-full">
            <strong className="block font-medium">{t("実行に失敗しました")}</strong>
            <span className="whitespace-pre-wrap break-words">{run.error}</span>
          </Alert>
        )}
      </div>
    );
  }

  return (
    <div className="group/msg flex gap-3">
      <span
        aria-hidden="true"
        className="mt-0.5 grid size-6 shrink-0 place-items-center rounded-md border border-line bg-gradient-to-b from-surface to-inset text-fg"
      >
        <Sparkles size={12} />
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <div
          className={cn(
            "flex items-center gap-1.5 text-xs text-faint",
            "[&_.actions]:ml-auto [&_.actions]:opacity-0",
            "group-hover/msg:[&_.actions]:opacity-100 focus-within:[&_.actions]:opacity-100",
          )}
        >
          <span className="truncate font-medium text-muted">
            {message.model_id ?? "assistant"}
          </span>
          <span>·</span>
          <span>{relativeTime(message.created_at)}</span>
          {run && run.tool_call_count > 0 && (
            <Badge
              asChild
              tone="info"
              size="sm"
            >
              <button
                type="button"
                onClick={() => {
                  setActivityRunId(run.id);
                  setRightPanel("activity");
                }}
              >
                <MessageSquare size={10} /> {run.tool_call_count} {t("件のツール")}
              </button>
            </Badge>
          )}
          <span className="actions">
            <CopyButton value={message.content} />
          </span>
        </div>
        {message.reasoning && <Reasoning text={message.reasoning} />}
        <div className="text-sm text-fg">
          <Markdown>{message.content}</Markdown>
        </div>
        {showRunError && run?.error && (
          <Alert tone="danger">
            <strong className="block font-medium">{t("実行に失敗しました")}</strong>
            <span className="whitespace-pre-wrap break-words">{run.error}</span>
          </Alert>
        )}
      </div>
    </div>
  );
});

function Reasoning({ text, streaming = false }: { text: string; streaming?: boolean }) {
  const [open, setOpen] = useState(streaming);
  useEffect(() => {
    if (streaming) setOpen(true);
  }, [streaming]);
  return (
    <details
      open={open}
      onToggle={(e) => setOpen((e.target as HTMLDetailsElement).open)}
      className="group rounded-xl border border-line bg-inset"
    >
      {/* `list-none` removes the native marker, so the disclosure state needs a
          affordance of its own: a chevron that rotates with `open`. */}
      <summary className="flex cursor-pointer list-none items-center gap-1.5 px-3 py-2 text-xs font-medium text-muted transition-colors hover:text-fg">
        <ChevronRight
          size={12}
          aria-hidden="true"
          className="shrink-0 transition-transform group-open:rotate-90"
        />
        <Brain size={12} className="shrink-0" aria-hidden="true" />
        {t("思考プロセス")}
      </summary>
      <div className="max-h-72 overflow-y-auto whitespace-pre-wrap border-t border-line px-3 py-2 text-xs leading-5 text-muted">
        {text}
      </div>
    </details>
  );
}

/** Compact, inline view of the tool calls the agent made in this run. The
 *  right-hand Activity panel has the full arguments/results; this keeps the
 *  conversation itself honest about what is happening while it streams. */
function InlineTools({ tools }: { tools: ToolActivity[] }) {
  return (
    <div className="flex flex-col overflow-hidden rounded-xl border border-line bg-inset/60">
      {tools.map((tool) => (
        <div
          key={tool.id}
          className="flex items-center gap-2 border-b border-line/60 px-2.5 py-1.5 text-xs last:border-b-0"
        >
          <Wrench size={12} className="shrink-0 text-faint" aria-hidden="true" />
          <span className="shrink-0 font-mono font-medium text-fg">{tool.name}</span>
          <span className="min-w-0 flex-1 truncate text-faint" title={describeToolCall(tool)}>
            {describeToolCall(tool)}
          </span>
          {tool.status === "running" ? (
            <Spinner size={11} />
          ) : tool.status === "error" ? (
            <CircleAlert size={12} className="shrink-0 text-danger" aria-hidden="true" />
          ) : (
            <Check size={12} className="shrink-0 text-success" aria-hidden="true" />
          )}
        </div>
      ))}
    </div>
  );
}

function LiveBlock({ live }: { live: RunTimeline }) {
  // Waiting on an approval is still "in flight": the spinner has to stay, and
  // the transcript must not be retired while a card is waiting for an answer.
  const busy = isBusy(live.status);
  return (
    <div className="flex gap-3">
      <span
        aria-hidden="true"
        className={cn(
          "mt-0.5 grid size-6 shrink-0 place-items-center rounded-md border border-line bg-gradient-to-b from-surface to-inset text-fg",
          busy && "animate-mix-pulse",
        )}
      >
        <Sparkles size={12} />
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        {live.error && (
          <Alert tone="danger">
            <strong className="block font-medium">{t("実行に失敗しました")}</strong>
            <span className="whitespace-pre-wrap break-words">{live.error}</span>
          </Alert>
        )}
        {live.stagnation && (
          <Alert tone="warning">
            <strong className="block font-medium">{t("停滞を検出しました")}</strong>
            <span>{live.stagnation}</span>
          </Alert>
        )}
        {live.verification && (
          <Alert tone="danger">
            <strong className="block font-medium">{t("検証が必要です")}</strong>
            <span>
              {tf("破壊的ツール {tool} が実行されました。確認の読み取りが期待されます。", {
                tool: live.verification.tool,
              })}
            </span>
          </Alert>
        )}
        {live.budgetRequest && <BudgetBanner live={live} />}
        {live.approval && <ApprovalCard live={live} />}
        {live.reasoning && (
          <Reasoning text={live.reasoning} streaming={busy && !live.text} />
        )}
        {live.tools.length > 0 && <InlineTools tools={live.tools} />}
        {live.text ? (
          <div className="text-sm text-fg">
            <Markdown>{live.text}</Markdown>
          </div>
        ) : (
          busy && (
            <div className="flex items-center gap-1.5 text-xs text-faint">
              <Spinner size={12} />
              {t("思考中…")}
            </div>
          )
        )}
        {live.verificationFailed && (
          <Alert tone="warning" hideIcon>
            <span className="inline-flex items-start gap-2">
              <ShieldAlert size={13} className="mt-0.5 shrink-0" />
              <span>{live.verificationFailed}</span>
            </span>
          </Alert>
        )}
      </div>
    </div>
  );
}

function BudgetBanner({ live }: { live: RunTimeline }) {
  const request = live.budgetRequest!;
  return (
    <Alert tone="warning" className="flex-col items-start">
      <strong className="block font-medium">{t("予算を使い切りました")}</strong>
      <span>
        {tf("ステップ上限 {steps} / ツール上限 {calls} に到達しました。", {
          steps: request.stepLimit ?? "?",
          calls: request.toolCallLimit ?? "?",
        })}{" "}
        {request.reason}
      </span>
      <div className="mt-2 flex items-center gap-2">
        <BudgetButtons runId={live.runId} request={request} />
      </div>
    </Alert>
  );
}

function BudgetButtons({
  runId,
  request,
}: {
  runId: string;
  request: { stepLimit?: number; toolCallLimit?: number };
}) {
  const [busy, setBusy] = useState(false);
  const decide = async (grant: boolean) => {
    setBusy(true);
    try {
      await api.budgetExtension({
        id: runId,
        grant,
        step_limit: grant ? (request.stepLimit ?? null) : null,
        tool_call_limit: grant ? (request.toolCallLimit ?? null) : null,
        wall_clock_minutes: grant ? 30 : null,
      });
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <Button variant="primary" size="sm" disabled={busy} onClick={() => void decide(true)}>
        {t("延長する")}
      </Button>
      <Button size="sm" disabled={busy} onClick={() => void decide(false)}>
        {t("ここで終了する")}
      </Button>
    </>
  );
}

function ApprovalCard({ live }: { live: RunTimeline }) {
  const approval = live.approval!;
  const [busy, setBusy] = useState(false);
  const decide = async (grant: boolean, remember: boolean) => {
    const id = approval.approvalId;
    if (!id) return;
    setBusy(true);
    try {
      await api.decideApproval({ id, grant, remember });
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="rounded-lg border border-warning bg-warning-soft">
      <div className="flex items-center gap-1.5 px-3 py-2 text-sm font-medium text-warning">
        <ShieldCheck size={15} className="shrink-0" />
        {t("承認が必要です")} — <code className="font-mono text-xs">{approval.name}</code>
      </div>
      <pre className="mx-3 max-h-44 overflow-auto rounded-md bg-surface/60 p-2 font-mono text-xs leading-5 text-muted">
        {JSON.stringify(approval.arguments, null, 2)}
      </pre>
      <div className="flex items-center justify-end gap-2 px-3 py-2">
        <Button size="sm" disabled={busy || !approval.approvalId} onClick={() => void decide(false, false)}>
          {t("拒否")}
        </Button>
        <Button size="sm" disabled={busy || !approval.approvalId} onClick={() => void decide(true, true)}>
          {t("常に許可")}
        </Button>
        <Button
          variant="primary"
          size="sm"
          disabled={busy || !approval.approvalId}
          onClick={() => void decide(true, false)}
        >
          {t("今回のみ許可")}
        </Button>
      </div>
    </div>
  );
}
