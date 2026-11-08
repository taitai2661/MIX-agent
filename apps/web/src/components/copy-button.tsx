import { Check, Copy, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { t } from "@/app/i18n";

type Status = "idle" | "copied" | "failed";

/**
 * Small clipboard control shared by messages and code blocks. The icon changes
 * for a moment so a copy that succeeded is visible without a toast.
 */
export function CopyButton({
  value,
  label,
  className = "",
}: {
  value: string | (() => string);
  label?: string;
  className?: string;
}) {
  const [status, setStatus] = useState<Status>("idle");
  const reset = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (reset.current !== null) window.clearTimeout(reset.current);
    },
    [],
  );
  const copy = useCallback(async () => {
    const text = typeof value === "function" ? value() : value;
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text);
      } else {
        const area = document.createElement("textarea");
        area.value = text;
        area.style.position = "fixed";
        area.style.opacity = "0";
        document.body.append(area);
        area.select();
        const ok = document.execCommand("copy");
        area.remove();
        if (!ok) throw new Error("copy failed");
      }
      setStatus("copied");
    } catch {
      setStatus("failed");
    }
    if (reset.current !== null) window.clearTimeout(reset.current);
    reset.current = window.setTimeout(() => setStatus("idle"), 1200);
  }, [value]);

  const text =
    status === "copied"
      ? t("コピーしました")
      : status === "failed"
        ? t("コピーできませんでした")
        : (label ?? t("コピー"));

  return (
    <button
      type="button"
      className={`message-action ${className}`}
      onClick={copy}
      title={text}
      aria-label={text}
    >
      {status === "copied" ? (
        <Check size={14} />
      ) : status === "failed" ? (
        <X size={14} />
      ) : (
        <Copy size={14} />
      )}
      <span className="sr-only" role="status">
        {status === "copied" ? t("コピーしました") : ""}
      </span>
    </button>
  );
}
