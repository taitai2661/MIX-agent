import { memo, useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  FileCode,
  FolderOpen,
  Terminal,
} from "lucide-react";
import { DiffLines, parseUnifiedDiff } from "./DiffView";
import { cn } from "@/lib/cn";
import { t, tf } from "@/lib/i18n";
import { payloadBlock } from "@/components/shared";

interface ApprovalDiffViewProps {
  toolName: string;
  args: Record<string, unknown> | null | undefined;
}

/** Characters of a file write rendered before the "show all" affordance. */
const PREVIEW_CHARS = 5_000;
/** Hard ceiling for an opt-in full render, so a generated file cannot freeze
 *  the approval card when the user asks to see everything. */
const FULL_CHARS = 200_000;
/** Raw JSON is collapsed to this length until the user asks for all of it. */
const RAW_PREVIEW_CHARS = 4_000;

interface BuiltDiff {
  path?: string;
  unified?: string;
  command?: string;
  cwd?: string;
  /** Characters withheld from the preview. 0/undefined when complete. */
  truncated?: number;
}

export function buildDiffFromArgs(
  toolName: string,
  args: Record<string, unknown>,
  options: { full?: boolean } = {},
): BuiltDiff | null {
  const full = options.full === true;
  // A large `files write` used to be sliced at 5,000 characters with no hint
  // that the card was showing only the start of the file the user was being
  // asked to authorise. The cap stays, but it is now visible and expandable.
  const bodyOf = (content: string) => {
    const cap = full ? FULL_CHARS : PREVIEW_CHARS;
    const body = content.slice(0, cap);
    return { body, truncated: Math.max(0, content.length - body.length) };
  };

  if (toolName === "terminal" && typeof args.command === "string") {
    return {
      command: args.command,
      cwd: typeof args.cwd === "string" ? args.cwd : undefined,
    };
  }

  if (toolName === "edit") {
    const path = typeof args.path === "string" ? args.path : "";
    if (Array.isArray(args.edits) && args.edits.length > 0) {
      let combined = "";
      for (const edit of args.edits as Record<string, unknown>[]) {
        const p = typeof edit.path === "string" ? edit.path : path;
        const oldStr =
          typeof edit.old_string === "string" ? edit.old_string : "";
        const newStr =
          typeof edit.new_string === "string" ? edit.new_string : "";
        // The hunk header is required by the parser but its absolute numbers
        // are unknowable from old/new strings. `DiffLines` is rendered without
        // line numbers below, so this is never shown to the user.
        combined += `--- a/${p}\n+++ b/${p}\n@@ -1,1 +1,1 @@\n`;
        for (const line of oldStr.split("\n")) {
          combined += `-${line}\n`;
        }
        for (const line of newStr.split("\n")) {
          combined += `+${line}\n`;
        }
      }
      return { path: path || "batch edits", unified: combined };
    }

    const action = typeof args.action === "string" ? args.action : "replace";
    const oldStr = typeof args.old_string === "string" ? args.old_string : "";
    const newStr = typeof args.new_string === "string" ? args.new_string : "";

    if (action === "create") {
      let combined = `--- /dev/null\n+++ b/${path}\n@@ -0,0 +1,1 @@\n`;
      for (const line of newStr.split("\n")) {
        combined += `+${line}\n`;
      }
      return { path, unified: combined };
    }

    if (action === "append") {
      let combined = `--- a/${path}\n+++ b/${path}\n@@ -end +end @@\n`;
      for (const line of newStr.split("\n")) {
        combined += `+${line}\n`;
      }
      return { path, unified: combined };
    }

    if (oldStr || newStr) {
      let combined = `--- a/${path}\n+++ b/${path}\n@@ -1,1 +1,1 @@\n`;
      for (const line of oldStr.split("\n")) {
        combined += `-${line}\n`;
      }
      for (const line of newStr.split("\n")) {
        combined += `+${line}\n`;
      }
      return { path, unified: combined };
    }
  }

  if (toolName === "files") {
    const action = typeof args.action === "string" ? args.action : "";
    const path = typeof args.path === "string" ? args.path : "";
    const content = typeof args.content === "string" ? args.content : "";

    if (action === "write" || action === "append") {
      const { body, truncated } = bodyOf(content);
      let combined = `--- a/${path}\n+++ b/${path}\n@@ -1,1 +1,1 @@\n`;
      for (const line of body.split("\n")) {
        combined += `+${line}\n`;
      }
      return { path, unified: combined, truncated };
    }
  }

  return null;
}

export const ApprovalDiffView = memo(function ApprovalDiffView({
  toolName,
  args,
}: ApprovalDiffViewProps) {
  const [showRaw, setShowRaw] = useState(false);
  const [showFullRaw, setShowFullRaw] = useState(false);
  const [showFullDiff, setShowFullDiff] = useState(false);

  const parsed = useMemo(() => {
    if (!args) return null;
    return buildDiffFromArgs(toolName, args, { full: showFullDiff });
  }, [toolName, args, showFullDiff]);

  const diffLines = useMemo(() => {
    if (!parsed?.unified) return null;
    return parseUnifiedDiff(parsed.unified);
  }, [parsed?.unified]);

  const rawArgs = JSON.stringify(args, null, 2) ?? "";
  const rawTruncated = !showFullRaw && rawArgs.length > RAW_PREVIEW_CHARS;
  const shownArgs = rawTruncated
    ? rawArgs.slice(0, RAW_PREVIEW_CHARS)
    : rawArgs;

  return (
    <div className="flex flex-col gap-2">
      {parsed?.command ? (
        <div className="rounded border border-line bg-card/60 p-2.5">
          <div className="mb-1 flex items-center gap-1.5 text-xs font-semibold text-fg-muted">
            <Terminal size={13} />
            <span>{t("実行コマンド")}</span>
          </div>
          {parsed.cwd && (
            <div className="mb-1 flex min-w-0 items-center gap-1 text-2xs text-faint">
              <FolderOpen size={11} className="shrink-0" aria-hidden="true" />
              <span className="truncate font-mono" title={parsed.cwd}>
                {parsed.cwd}
              </span>
            </div>
          )}
          <div className="overflow-x-auto rounded bg-inset p-2 font-mono text-xs text-brand">
            $ {parsed.command}
          </div>
        </div>
      ) : parsed?.unified && diffLines && diffLines.length > 0 ? (
        <div className="flex flex-col gap-1.5 rounded border border-line bg-card/60 p-2">
          {parsed.path && (
            <div className="flex items-center gap-1.5 px-1 text-xs font-medium text-fg-muted">
              <FileCode size={13} />
              <span className="truncate font-mono" title={parsed.path}>
                {parsed.path}
              </span>
            </div>
          )}
          <div className="max-h-60 overflow-y-auto rounded bg-inset">
            {/* No line-number gutter: the approval payload carries old/new
                strings, not the file, so any absolute number would be made up. */}
            <DiffLines lines={diffLines} showLineNumbers={false} />
          </div>
          {!!parsed.truncated && (
            <div className="flex flex-wrap items-center justify-between gap-2 px-1 text-2xs text-warning">
              <span>
                {tf("先頭{shown}文字のみ表示（残り{rest}文字）", {
                  shown: PREVIEW_CHARS.toLocaleString(),
                  rest: parsed.truncated.toLocaleString(),
                })}
              </span>
              <button
                type="button"
                onClick={() => setShowFullDiff(true)}
                className="rounded px-1.5 py-0.5 font-medium text-brand hover:bg-brand-soft"
              >
                {t("全文を表示")}
              </button>
            </div>
          )}
        </div>
      ) : null}

      <div className="px-1">
        <button
          type="button"
          onClick={() => setShowRaw((prev) => !prev)}
          className="flex items-center gap-1 text-xs text-fg-muted transition hover:text-fg"
        >
          {showRaw ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
          <span>{t("引数の詳細 (JSON)")}</span>
        </button>
        {showRaw && (
          <div className="mt-1.5 flex flex-col gap-1">
            <pre className={cn(payloadBlock, "max-h-40 break-all")}>
              {shownArgs}
            </pre>
            {rawTruncated && (
              <div className="flex items-center justify-between gap-2 text-2xs text-faint">
                <span>{t("先頭4,000文字のみ表示")}</span>
                <button
                  type="button"
                  onClick={() => setShowFullRaw(true)}
                  className="rounded px-1.5 py-0.5 font-medium text-brand hover:bg-brand-soft"
                >
                  {t("全文を表示")}
                </button>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
});
