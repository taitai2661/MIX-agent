import { memo, useMemo, useState } from "react";
import { Check, Copy, Timer } from "lucide-react";
import { t } from "@/lib/i18n";
import { Badge } from "@/components/ui/badge";
import { AnsiText } from "@/components/AnsiText";

interface TerminalResultViewProps {
  result: Record<string, unknown> | null | undefined;
}

export const TerminalResultView = memo(function TerminalResultView({
  result,
}: TerminalResultViewProps) {
  const [copied, setCopied] = useState(false);

  const res = result && typeof result === "object" ? result : null;
  const stdout = typeof res?.stdout === "string" ? res.stdout : "";
  const stderr = typeof res?.stderr === "string" ? res.stderr : "";
  const exitCode = typeof res?.exit_code === "number" ? res.exit_code : null;
  const startedAt =
    typeof res?.started_at === "string"
      ? new Date(res.started_at).getTime()
      : null;
  const finishedAt =
    typeof res?.finished_at === "string"
      ? new Date(res.finished_at).getTime()
      : null;

  const durationMs =
    startedAt && finishedAt && finishedAt >= startedAt
      ? finishedAt - startedAt
      : null;

  const formattedDuration = useMemo(() => {
    if (durationMs === null) return null;
    if (durationMs < 1000) return `${durationMs}ms`;
    return `${(durationMs / 1000).toFixed(1)}s`;
  }, [durationMs]);

  const outputText = useMemo(
    () => [stdout, stderr].filter(Boolean).join("\n").trim(),
    [stdout, stderr],
  );

  if (!res) return null;

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
    <div className="flex flex-col gap-1.5 rounded-lg border border-line bg-surface p-2 text-xs">
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
            className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-faint transition hover:bg-hover hover:text-fg"
            title={t("出力をコピー")}
          >
            {copied ? (
              <>
                <Check size={12} className="text-success" />
                <span className="text-2xs text-success">{t("コピー済み")}</span>
              </>
            ) : (
              <>
                <Copy size={12} />
                <span className="text-2xs">{t("コピー")}</span>
              </>
            )}
          </button>
        )}
      </div>

      <div className="max-h-64 overflow-x-auto overflow-y-auto rounded-md bg-inset p-2 font-mono leading-relaxed whitespace-pre-wrap break-all">
        <AnsiText
          text={outputText}
          empty={<span className="text-faint">{t("（出力なし）")}</span>}
        />
      </div>
    </div>
  );
});
