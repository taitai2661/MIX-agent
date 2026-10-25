import { useCallback, useEffect, useRef, useState } from "react";
import { Check, Copy, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { t, tf } from "@/lib/i18n";
import { copyText } from "@/lib/clipboard";
import { Button } from "./ui/button";

export function CopyButton({
  value,
  label,
  className,
}: {
  value: string;
  label?: string;
  className?: string;
}) {
  const [status, setStatus] = useState<"idle" | "copied" | "failed">("idle");
  // Cleared on unmount: a copy button that unmounts inside the 1.2s window
  // used to call setState on a component that is no longer mounted.
  const reset = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (reset.current !== null) window.clearTimeout(reset.current);
    },
    [],
  );
  const onCopy = useCallback(async () => {
    const ok = await copyText(value);
    setStatus(ok ? "copied" : "failed");
    if (reset.current !== null) window.clearTimeout(reset.current);
    reset.current = window.setTimeout(() => setStatus("idle"), 1200);
  }, [value]);

  const copied = status === "copied";
  const failed = status === "failed";

  return (
    <Button
      variant="ghost"
      size="icon-sm"
      className={cn("shrink-0 text-faint hover:text-fg", className)}
      onClick={onCopy}
      title={failed ? t("コピーできませんでした") : (label ?? t("コピー"))}
      aria-label={failed ? t("コピーできませんでした") : (label ?? t("コピー"))}
    >
      {copied ? (
        <Check size={13} className="text-success" />
      ) : failed ? (
        <X size={13} className="text-danger" />
      ) : (
        <Copy size={13} />
      )}
      <span aria-live="polite" className="sr-only">
        {copied
          ? t("コピーしました")
          : failed
            ? t("コピーできませんでした")
            : ""}
      </span>
    </Button>
  );
}

/** Fenced-code renderer body: header with language + copy button. */
export function CodeBlock({
  language,
  code,
}: {
  language: string;
  code: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const lineCount = code.split("\n").length;
  const collapsible = lineCount > 30 || code.length > 6000;
  const collapsed = collapsible && !expanded;
  return (
    <div className="my-2.5 overflow-hidden rounded-lg border border-line bg-inset shadow-xs">
      <div className="flex items-center justify-between gap-2 border-b border-line bg-surface/70 px-3 py-1.5">
        <span className="min-w-0 truncate font-mono text-2xs tracking-wide text-muted uppercase">
          {language || "text"}
        </span>
        <span className="flex shrink-0 items-center gap-1">
          {collapsible && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setExpanded((v) => !v)}
              aria-expanded={expanded}
              className="h-6 px-2 text-2xs"
            >
              {expanded
                ? t("折りたたむ")
                : tf("すべて表示 ({n})", { n: lineCount })}
            </Button>
          )}
          <CopyButton value={code} label={t("コピー")} />
        </span>
      </div>
      <pre
        className={cn(
          "overflow-x-auto px-3.5 py-3",
          collapsed && "max-h-80 overflow-y-auto",
        )}
      >
        <code className="font-mono text-xs leading-5 text-fg">{code}</code>
      </pre>
    </div>
  );
}
