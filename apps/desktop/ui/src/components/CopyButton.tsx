import { useCallback, useEffect, useRef, useState } from "react";
import { Check, Copy, Save, X } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { copyText } from "@/lib/clipboard";
import { useToasts } from "@/lib/toast";
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

/**
 * File extension for a fenced-code language tag. Unknown languages still get
 * a usable `.txt` name rather than an extensionless file the OS cannot open.
 */
export function extensionForLanguage(language: string): string {
  const key = language.trim().toLowerCase();
  const aliases: Record<string, string> = {
    js: "js",
    javascript: "js",
    mjs: "mjs",
    cjs: "cjs",
    ts: "ts",
    typescript: "ts",
    tsx: "tsx",
    jsx: "jsx",
    py: "py",
    python: "py",
    rb: "rb",
    ruby: "rb",
    rs: "rs",
    rust: "rs",
    go: "go",
    golang: "go",
    java: "java",
    kt: "kt",
    kotlin: "kt",
    swift: "swift",
    c: "c",
    h: "h",
    cpp: "cpp",
    "c++": "cpp",
    cs: "cs",
    csharp: "cs",
    php: "php",
    sh: "sh",
    bash: "sh",
    zsh: "sh",
    shell: "sh",
    fish: "fish",
    ps1: "ps1",
    powershell: "ps1",
    sql: "sql",
    json: "json",
    jsonc: "jsonc",
    yaml: "yaml",
    yml: "yml",
    toml: "toml",
    xml: "xml",
    html: "html",
    css: "css",
    scss: "scss",
    less: "less",
    md: "md",
    markdown: "md",
    diff: "diff",
    patch: "patch",
    dockerfile: "dockerfile",
  };
  return aliases[key] ?? "txt";
}

/** Fenced-code renderer body: header with language + copy/save buttons. */
export function CodeBlock({
  language,
  code,
}: {
  language: string;
  code: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const [saving, setSaving] = useState(false);
  const pushToast = useToasts((s) => s.push);
  const lineCount = code.split("\n").length;
  const collapsible = lineCount > 30 || code.length > 6000;
  const collapsed = collapsible && !expanded;

  const onSave = useCallback(async () => {
    if (saving) return;
    setSaving(true);
    try {
      const extension = extensionForLanguage(language);
      const path = await api.pickSaveFile({
        title: t("コードを保存"),
        defaultName: `snippet.${extension}`,
        filters: [{ name: extension.toUpperCase(), extensions: [extension] }],
      });
      if (!path) return;
      await api.saveTextFile(path, code);
      pushToast({ message: t("保存しました") });
    } catch (error) {
      pushToast({
        message: tf("保存できませんでした: {reason}", {
          reason: errorMessage(error),
        }),
      });
    } finally {
      setSaving(false);
    }
  }, [code, language, pushToast, saving]);

  return (
    <div className="my-2.5 overflow-hidden rounded-lg border border-line bg-inset shadow-xs">
      <div className="flex items-center justify-between gap-2 border-b border-line bg-surface px-3 py-1.5">
        <span className="min-w-0 truncate font-mono text-2xs tracking-wide text-muted uppercase">
          {language || "text"}
        </span>
        <span className="flex shrink-0 items-center gap-1">
          {collapsible && (
            <Button
              variant="ghost"
              size="xs"
              onClick={() => setExpanded((v) => !v)}
              aria-expanded={expanded}
            >
              {expanded
                ? t("折りたたむ")
                : tf("すべて表示 ({n})", { n: lineCount })}
            </Button>
          )}
          <Button
            variant="ghost"
            size="icon-sm"
            className="shrink-0 text-faint hover:text-fg"
            onClick={() => void onSave()}
            disabled={saving}
            title={t("ファイルに保存")}
            aria-label={t("ファイルに保存")}
          >
            <Save size={13} />
          </Button>
          <CopyButton value={code} label={t("コピー")} />
        </span>
      </div>
      <pre
        className={cn(
          "code-block-pre overflow-x-auto px-3.5 py-3",
          collapsed && "max-h-80 overflow-y-auto",
        )}
      >
        <code className="font-mono text-xs leading-5 text-fg">{code}</code>
      </pre>
    </div>
  );
}
