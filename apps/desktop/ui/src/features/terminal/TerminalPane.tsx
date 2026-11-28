import { useEffect, useRef, useState } from "react";
import {
  CornerDownLeft,
  Eraser,
  Play,
  Square,
  SquareTerminal,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { useActiveRoot } from "@/lib/queries";
import { EmptyState } from "@/components/shared";
import { AnsiText } from "@/components/AnsiText";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

interface Entry {
  command: string;
  output: string;
  exitCode: number | null;
  cwd: string | null;
  at: string;
  error?: string;
}

interface Live {
  command: string;
  output: string;
  at: string;
  cwd: string | null;
}

const POLL_MS = 150;

/**
 * Run commands in the workspace using the same persistent shell as the agent.
 *
 * The command starts without waiting (`start_shell_command`) and the pane polls
 * `poll_shell_command` for the output between the shell's protocol markers.
 * That is what makes a long build visible while it runs, and it also means the
 * pane can re-attach after a tab switch: the backend keeps the in-flight state,
 * not the component. Ctrl-C writes `\x03` to the PTY, which is the same SIGINT
 * a real terminal sends.
 */
export function TerminalPane() {
  const root = useActiveRoot();
  const [entries, setEntries] = useState<Entry[]>([]);
  const [live, setLive] = useState<Live | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pollEpoch, setPollEpoch] = useState(0);
  const endRef = useRef<HTMLDivElement | null>(null);
  const liveRef = useRef<Live | null>(null);
  liveRef.current = live;

  useEffect(() => {
    if (!root) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // The first poll re-attaches to a command started before a tab switch.
    let command = liveRef.current?.command ?? null;

    const tick = async () => {
      let stop = false;
      try {
        const result = await api.pollShellCommand(root);
        if (cancelled) return;
        if (result.idle) {
          // A command finished elsewhere (or the shell was reset).
          setLive((previous) => (previous ? null : previous));
          stop = true;
          return;
        }
        const nextCommand =
          result.command ?? command ?? liveRef.current?.command ?? "";
        command = nextCommand;
        if (result.done) {
          const finished: Entry = {
            command: nextCommand,
            output: result.output,
            exitCode: result.exit_code,
            cwd: result.cwd,
            at: new Date().toISOString(),
            ...(result.error ? { error: result.error } : {}),
          };
          setEntries((previous) => [...previous, finished]);
          setLive(null);
          stop = true;
          return;
        }
        setLive((previous) => ({
          command: nextCommand,
          output: result.output,
          at: previous?.at ?? new Date().toISOString(),
          cwd: result.cwd ?? previous?.cwd ?? null,
        }));
      } catch (err) {
        if (cancelled) return;
        setError(errorMessage(err) || t("エラー"));
        // A single transient IPC failure used to kill live output permanently;
        // the next tick recovers it (starting a new command also resets the
        // epoch and restarts this effect).
      } finally {
        if (!cancelled && !stop) timer = setTimeout(tick, POLL_MS);
      }
    };
    timer = setTimeout(tick, 50);
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [root, pollEpoch]);

  useEffect(() => {
    // jsdom (and some embedded WebViews) do not implement scrollIntoView.
    endRef.current?.scrollIntoView?.({ block: "end" });
  }, [entries, live?.output]);

  if (!root) {
    return (
      <EmptyState
        icon={<SquareTerminal size={22} />}
        title={t("ターミナルを使うにはプロジェクトを開いてください")}
        sub={t(
          "ワークスペースのフォルダを開くと、その場所でコマンドを実行できます。",
        )}
      />
    );
  }

  const start = async () => {
    const value = draft.trim();
    if (!value || busy) return;
    setBusy(true);
    setError(null);
    try {
      const started = await api.startShellCommand({
        root,
        command: value,
        max_wall_clock_ms: 600_000,
      });
      setLive({
        command: value,
        output: "",
        at: new Date().toISOString(),
        cwd: started.cwd,
      });
      setDraft("");
      setPollEpoch((epoch) => epoch + 1);
    } catch (err) {
      setError(errorMessage(err) || t("エラー"));
    } finally {
      setBusy(false);
    }
  };

  const send = async () => {
    if (!liveRef.current) return;
    const value = draft;
    setDraft("");
    setError(null);
    try {
      // The response is ignored: the next poll sees the echo and the output.
      await api.sendShellInput({ root, input: value, wait_ms: 0 });
    } catch (err) {
      setError(errorMessage(err) || t("エラー"));
    }
  };

  const interrupt = async () => {
    setError(null);
    try {
      await api.interruptShellCommand(root);
    } catch (err) {
      setError(errorMessage(err) || t("エラー"));
    }
  };

  const running = live !== null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-line px-3 py-1.5 text-2xs text-muted">
        <SquareTerminal size={13} className="shrink-0 text-brand" />
        <code className="min-w-0 flex-1 truncate" title={root}>
          {root}
        </code>
        <Button
          variant="ghost"
          size="xs"
          className="gap-1"
          disabled={entries.length === 0 || busy}
          onClick={() => setEntries([])}
          title={t("表示をクリア")}
        >
          <Eraser size={12} aria-hidden="true" />
          {t("クリア")}
        </Button>
      </div>

      <div className="stable-scroll min-h-0 flex-1 overflow-y-auto px-3 py-2 font-mono text-xs">
        {entries.length === 0 && !live && (
          <p className="m-0 text-faint">
            {t(
              "エージェントと同じ永続シェルでコマンドを実行します。cd と環境変数は次のコマンドにも引き継がれます。",
            )}
          </p>
        )}
        {entries.map((entry, index) => (
          <div key={`${entry.at}-${index}`} className="mb-3 last:mb-0">
            <div className="flex items-start gap-1.5 text-fg">
              <span className="select-none text-brand">$</span>
              <code className="min-w-0 flex-1 break-all">{entry.command}</code>
              {entry.exitCode !== null && (
                <Badge
                  tone={entry.exitCode === 0 ? "success" : "danger"}
                  size="sm"
                  className="shrink-0 font-mono"
                >
                  {entry.exitCode}
                </Badge>
              )}
            </div>
            {entry.output.trim() !== "" && (
              <pre className="mt-1 mb-0 whitespace-pre-wrap break-words text-muted">
                <AnsiText text={entry.output} />
              </pre>
            )}
            {entry.error && (
              <p className="mt-1 mb-0 text-2xs text-danger">{entry.error}</p>
            )}
          </div>
        ))}

        {live && (
          <div className="mb-3">
            <div className="flex items-center gap-1.5 text-fg">
              <span className="select-none text-brand">$</span>
              <code className="min-w-0 flex-1 break-all">{live.command}</code>
              <Button
                variant="ghost-danger"
                size="xs"
                className="shrink-0 gap-1 px-1.5 text-2xs"
                onClick={() => void interrupt()}
                title={t("Ctrl-C を送信")}
              >
                <Square size={10} aria-hidden="true" />
                {t("停止")}
              </Button>
            </div>
            {live.output.trim() !== "" && (
              <pre className="mt-1 mb-0 whitespace-pre-wrap break-words text-muted">
                <AnsiText text={live.output} />
              </pre>
            )}
            <p className="mt-1 mb-0 flex items-center gap-1.5 text-faint">
              <span className="animate-mix-pulse" aria-hidden="true">
                ●
              </span>
              {t("実行中…")}
            </p>
          </div>
        )}
        <div ref={endRef} />
      </div>

      {error && (
        <p
          role="alert"
          className="mx-3 mb-1 rounded-md bg-danger-soft px-2 py-1 text-xs text-danger"
        >
          {error}
        </p>
      )}

      <form
        className="flex shrink-0 items-center gap-1.5 border-t border-line px-3 py-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (running) void send();
          else void start();
        }}
      >
        <span className="font-mono text-sm text-brand" aria-hidden="true">
          $
        </span>
        <input
          className="min-w-0 flex-1 bg-transparent font-mono text-xs text-fg outline-none placeholder:text-faint"
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          placeholder={
            running
              ? t("実行中のコマンドへ入力（Enter で送信）")
              : t("コマンドを入力（例: git status）")
          }
          aria-label={
            running ? t("実行中のコマンドへの入力") : t("実行するコマンド")
          }
          spellCheck={false}
          autoComplete="off"
          disabled={busy}
        />
        <Button
          type="submit"
          variant="primary"
          size="icon-sm"
          disabled={busy || (!running && draft.trim() === "")}
          title={running ? t("入力を送信") : t("実行")}
          aria-label={running ? t("入力を送信") : t("実行")}
        >
          {running ? <CornerDownLeft size={13} /> : <Play size={12} />}
        </Button>
      </form>
      <p className="mx-3 mb-2 text-2xs text-faint">
        {running
          ? t(
              "入力は実行中のプロセスへそのまま送られます。停止は Ctrl-C（停止ボタン）です。",
            )
          : `${tf("最大 {seconds} 秒で打ち切ります。", { seconds: 600 })} ${t(
              "承認なしで実行されます。内容を確認してから送信してください。",
            )}`}
      </p>
    </div>
  );
}
