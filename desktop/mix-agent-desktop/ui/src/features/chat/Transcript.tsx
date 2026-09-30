import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  ArrowDown, BookOpen, Brain, FileText, MessageSquare, Search, ShieldAlert, ShieldCheck, Sparkles, Terminal, Wand2,
} from "lucide-react";
import { Markdown } from "@/components/Markdown";
import { CopyButton } from "@/components/CopyButton";
import { t, tf } from "@/lib/i18n";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { relativeTime } from "@/lib/format";
import { api } from "@/lib/api";
import { isBusy, type RunTimeline } from "@/lib/runEvents";
import type { MessageView, RunView } from "@/lib/types";
import { useUi } from "@/lib/ui";
import { Alert, Skeleton } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

export function Transcript({
  messages,
  runs,
  live,
  liveRunId,
  loading = false,
  error = null,
}: {
  messages: MessageView[];
  runs: Map<string, RunView>;
  live: RunTimeline | null;
  liveRunId: string | null;
  loading?: boolean;
  /** Set when the message history could not be loaded — without it a failed
   * fetch rendered as "no messages yet", i.e. as an empty conversation. */
  error?: unknown;
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
  const isEmpty = messages.length === 0 && !showLive;

  return (
    <div className="relative min-h-0 flex-1">
      <div ref={scrollRef} onScroll={onScroll} className="h-full overflow-y-auto">
        <div
          className="mx-auto flex w-full max-w-[820px] flex-col gap-6 px-6 py-6"
          role="log"
          aria-live="polite"
          aria-relevant="additions text"
          aria-label={t("会話")}
        >
          {error != null && (
            <Alert tone="danger">{errorMessage(error) || t("エラー")}</Alert>
          )}
          {isEmpty && !error && <Welcome />}
          {!isEmpty && loading && messages.length === 0 && !showLive && (
            <Skeleton className="h-16 w-full" />
          )}
          {messages.map((message) => (
            <MessageItem key={message.id} message={message} runs={runs} />
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
          className="absolute bottom-4 left-1/2 -translate-x-1/2 shadow-md"
        >
          <ArrowDown size={13} /> {t("最新へ")}
        </Button>
      )}
    </div>
  );
}

function Welcome() {
  return (
    <div className="flex flex-col items-center gap-5 py-12 text-center">
      <div className="grid size-12 place-items-center rounded-2xl border border-line bg-gradient-to-b from-surface to-inset text-fg shadow-xs">
        <Sparkles size={22} />
      </div>
      <div className="flex flex-col gap-1.5">
        <h2 className="text-2xl font-semibold tracking-tight text-fg">MIX-agent</h2>
        <p className="mx-auto max-w-md text-sm text-muted">
          {t("何でも質問してください。/ でコマンド、⌘K で検索。")}
        </p>
      </div>
      <QuickPrompts />
      <div className="flex flex-wrap items-center justify-center gap-x-4 gap-y-1 text-2xs text-faint">
        <span className="inline-flex items-center gap-1">
          <kbd className="rounded-sm border border-line bg-surface px-1 font-mono text-2xs leading-4">Enter</kbd>
          {t("送信")}
        </span>
        <span className="inline-flex items-center gap-1">
          <kbd className="rounded-sm border border-line bg-surface px-1 font-mono text-2xs leading-4">⇧</kbd>
          +
          <kbd className="rounded-sm border border-line bg-surface px-1 font-mono text-2xs leading-4">Enter</kbd>
          {t("改行")}
        </span>
        <span className="inline-flex items-center gap-1">
          <kbd className="rounded-sm border border-line bg-surface px-1 font-mono text-2xs leading-4">⌘K</kbd>
          {t("コマンドパレット")}
        </span>
      </div>
    </div>
  );
}

interface QuickPrompt {
  id: string;
  icon: typeof Wand2;
  text: string;
  mode?: "chat" | "thinking" | "agent";
}

const QUICK_PROMPTS: QuickPrompt[] = [
  { id: "explain", icon: Wand2, text: "このコードの意図と動作を、日本語で要約して", mode: "thinking" },
  { id: "refactor", icon: Terminal, text: "リファクタ案を出して。安全策もセットで" },
  { id: "search", icon: Search, text: "リポジトリ内の TODO を全部集めて" },
  { id: "docs", icon: BookOpen, text: "このプロジェクトの README 草案を書いて" },
];

function QuickPrompts() {
  return (
    <div className="grid w-full max-w-xl grid-cols-1 gap-2 sm:grid-cols-2">
      {QUICK_PROMPTS.map((p) => {
        const Icon = p.icon;
        return (
          <button
            key={p.id}
            type="button"
            onClick={() => {
              window.dispatchEvent(new CustomEvent("mix:insert-prompt", {
                detail: { text: p.text, mode: p.mode },
              }));
            }}
            className={cn(
              "group flex items-start gap-2 rounded-lg border border-line bg-surface px-3 py-2.5 text-start",
              "transition-colors duration-100 hover:border-line-strong hover:bg-hover",
              "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-focus",
            )}
          >
            <span className="mt-0.5 grid size-6 shrink-0 place-items-center rounded-md bg-inset text-muted transition-colors group-hover:bg-surface group-hover:text-fg">
              <Icon size={12} aria-hidden="true" />
            </span>
            <span className="min-w-0 flex-1 text-xs leading-5 text-fg">{t(p.text)}</span>
          </button>
        );
      })}
    </div>
  );
}

/** Memoised: the list renders every message on each token of a stream, and a
 * message only depends on its own row plus the run it belongs to. */
const MessageItem = memo(function MessageItem({
  message,
  runs,
}: {
  message: MessageView;
  runs: Map<string, RunView>;
}) {
  const setActivityRunId = useUi((s) => s.setActivityRunId);
  const setRightPanel = useUi((s) => s.setRightPanel);

  if (message.role === "user") {
    return (
      <div className="flex flex-col items-end gap-1">
        {message.attachments.length > 0 && (
          <div className="flex flex-wrap justify-end gap-1.5">
            {message.attachments.map((a, i) => (
              <span
                key={i}
                className="inline-flex items-center gap-1 rounded-md border border-line bg-inset px-1.5 py-0.5 text-2xs text-muted"
              >
                <FileText size={11} />
                {a.filename ?? a.mime_type}
              </span>
            ))}
          </div>
        )}
        <div className="max-w-[85%] whitespace-pre-wrap rounded-2xl rounded-br-md bg-inset px-3.5 py-2 text-sm leading-6 text-fg">
          {message.content}
        </div>
        <div className="px-1 text-2xs text-faint">{relativeTime(message.created_at)}</div>
      </div>
    );
  }

  const run = message.run_id ? runs.get(message.run_id) : undefined;
  return (
    <div className="flex flex-col gap-1.5">
      <div
        className={cn(
          "group flex items-center gap-1.5 text-2xs text-faint",
          "[&_.actions]:ml-auto [&_.actions]:opacity-0",
          "hover:[&_.actions]:opacity-100 focus-within:[&_.actions]:opacity-100",
        )}
      >
        <Brain size={11} className="shrink-0" />
        <span className="truncate">{message.model_id ?? "assistant"}</span>
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
      className="group rounded-lg border border-line bg-inset"
    >
      <summary className="flex cursor-pointer list-none items-center gap-1.5 px-3 py-1.5 text-2xs font-medium text-muted transition-colors hover:text-fg">
        <Brain size={12} className="shrink-0" />
        {t("思考プロセス")}
      </summary>
      <div className="max-h-72 overflow-y-auto whitespace-pre-wrap border-t border-line px-3 py-2 text-xs leading-5 text-muted">
        {text}
      </div>
    </details>
  );
}

function LiveBlock({ live }: { live: RunTimeline }) {
  // Waiting on an approval is still "in flight": the spinner has to stay, and
  // the transcript must not be retired while a card is waiting for an answer.
  const busy = isBusy(live.status);
  return (
    <div className="flex flex-col gap-1.5">
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
      {live.text ? (
        <div className="text-sm text-fg">
          <Markdown>{live.text}</Markdown>
        </div>
      ) : (
        busy && (
          <div className="flex items-center gap-1.5 text-xs text-faint">
            <span className="spin inline-flex">○</span>
            {t("思考中…")}
          </div>
        )
      )}
      {live.verificationFailed && (
        <Alert tone="warning">
          <ShieldAlert size={13} className="mt-0.5 shrink-0" />
          <span>{live.verificationFailed}</span>
        </Alert>
      )}
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
      <pre className="mx-3 max-h-44 overflow-auto rounded-md bg-surface/60 p-2 font-mono text-2xs leading-5 text-muted">
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
