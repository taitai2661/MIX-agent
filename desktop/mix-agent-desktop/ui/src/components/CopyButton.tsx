import { useCallback, useState } from "react";
import { Check, Copy } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
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
  const [copied, setCopied] = useState(false);
  const onCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      // Clipboard may be unavailable; the visual feedback still resets.
    }
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1200);
  }, [value]);

  return (
    <Button
      variant="ghost"
      size="icon-sm"
      className={cn("text-faint", className)}
      onClick={onCopy}
      title={label ?? t("Copy")}
      aria-label={label ?? t("Copy")}
    >
      {copied ? <Check size={13} className="text-success" /> : <Copy size={13} />}
    </Button>
  );
}

/** Fenced-code renderer body: header with language + copy button. */
export function CodeBlock({ language, code }: { language: string; code: string }) {
  return (
    <div className="my-2.5 overflow-hidden rounded-lg border border-line bg-inset">
      <div className="flex items-center justify-between border-b border-line px-3 py-1">
        <span className="font-mono text-2xs text-faint">{language || "text"}</span>
        <CopyButton value={code} label={t("Copy")} />
      </div>
      <pre className="overflow-x-auto px-3 py-2.5">
        <code className="font-mono text-xs leading-5 text-fg">{code}</code>
      </pre>
    </div>
  );
}
