import { memo, useMemo, useState } from "react";
import { Check, Copy, Terminal as TerminalIcon, Timer } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { Badge } from "@/components/ui/badge";

interface TerminalResultViewProps {
  result: Record<string, unknown> | null | undefined;
}

interface AnsiSpan {
  text: string;
  className: string;
}

// Convert common ANSI escape codes into CSS classes
function parseAnsi(raw: string): AnsiSpan[] {
  // Strip non-color escape sequences (e.g. cursor moves, clear screen)
  const cleaned = raw.replace(/\x1b\[[0-9;]*[A-HJKST]/g, "");

  const parts = cleaned.split(/(\x1b\[[0-9;]*m)/g);
  const spans: AnsiSpan[] = [];
  let currentClasses: string[] = [];

  for (const part of parts) {
    if (!part) continue;
    const match = part.match(/^\x1b\[([0-9;]*)m$/);
    if (match) {
      const codes = (match[1] || "0").split(";").map(Number);
      for (const code of codes) {
        if (code === 0) {
          currentClasses = [];
        } else if (code === 1) {
          if (!currentClasses.includes("font-bold")) currentClasses.push("font-bold");
        } else if (code === 31 || code === 91) {
          currentClasses = currentClasses.filter((c) => !c.startsWith("text-"));
          currentClasses.push("text-danger");
        } else if (code === 32 || code === 92) {
          currentClasses = currentClasses.filter((c) => !c.startsWith("text-"));
          currentClasses.push("text-success");
        } else if (code === 33 || code === 93) {
          currentClasses = currentClasses.filter((c) => !c.startsWith("text-"));
          currentClasses.push("text-warning");
        } else if (code === 34 || code === 94) {
          currentClasses = currentClasses.filter((c) => !c.startsWith("text-"));
          currentClasses.push("text-brand");
        } else if (code === 35 || code === 95) {
          currentClasses = currentClasses.filter((c) => !c.startsWith("text-"));
          currentClasses.push("text-purple-400");
        } else if (code === 36 || code === 96) {
          currentClasses = currentClasses.filter((c) => !c.startsWith("text-"));
          currentClasses.push("text-cyan-400");
        } else if (code === 37 || code === 97) {
          currentClasses = currentClasses.filter((c) => !c.startsWith("text-"));
          currentClasses.push("text-fg");
        }
      }
    } else {
      spans.push({ text: part, className: currentClasses.join(" ") });
    }
  }

  return spans;
}

export const TerminalResultView = memo(function TerminalResultView({
  result,
}: TerminalResultViewProps) {
  const [copied, setCopied] = useState(false);

  if (!result || typeof result !== "object") return null;

  const stdout = typeof result.stdout === "string" ? result.stdout : "";
  const stderr = typeof result.stderr === "string" ? result.stderr : "";
  const exitCode = typeof result.exit_code === "number" ? result.exit_code : null;
  const startedAt = typeof result.started_at === "string" ? new Date(result.started_at).getTime() : null;
  const finishedAt = typeof result.finished_at === "string" ? new Date(result.finished_at).getTime() : null;

  const durationMs =
    startedAt && finishedAt && finishedAt >= startedAt
      ? finishedAt - startedAt
      : null;

  const formattedDuration = useMemo(() => {
    if (durationMs === null) return null;
    if (durationMs < 1000) return `${durationMs}ms`;
    return `${(durationMs / 1000).toFixed(1)}s`;
  }, [durationMs]);

  const outputText = [stdout, stderr].filter(Boolean).join("\n").trim();
  const spans = useMemo(() => parseAnsi(outputText), [outputText]);

  const copyOutput = async () => {
    try {
      await navigator.clipboard.writeText(outputText);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // clipboard write failed
    }
  };

  return (
    <div className="flex flex-col gap-1.5 rounded border border-line bg-card/50 p-2 text-xs">
      <div className="flex flex-wrap items-center justify-between gap-1.5 border-b border-line pb-1.5">
        <div className="flex items-center gap-2">
          {exitCode !== null && (
            <Badge tone={exitCode === 0 ? "success" : "danger"} size="sm">
              exit {exitCode}
            </Badge>
          )}
          {formattedDuration && (
            <span className="flex items-center gap-1 font-mono text-faint">
              <Timer size={11} />
              {formattedDuration}
            </span>
          )}
        </div>
        {outputText && (
          <button
            type="button"
            onClick={copyOutput}
            className="flex items-center gap-1 rounded px-1.5 py-0.5 text-faint transition hover:bg-hover hover:text-fg"
            title={t("出力をコピー")}
          >
            {copied ? (
              <>
                <Check size={12} className="text-success" />
                <span className="text-[11px] text-success">{t("コピー済み")}</span>
              </>
            ) : (
              <>
                <Copy size={12} />
                <span className="text-[11px]">{t("コピー")}</span>
              </>
            )}
          </button>
        )}
      </div>

      <div className="max-h-64 overflow-x-auto overflow-y-auto rounded bg-inset p-2 font-mono leading-relaxed whitespace-pre-wrap break-all">
        {spans.length > 0 ? (
          spans.map((span, idx) => (
            <span key={idx} className={span.className || undefined}>
              {span.text}
            </span>
          ))
        ) : (
          <span className="text-faint">{t("（出力なし）")}</span>
        )}
      </div>
    </div>
  );
});
