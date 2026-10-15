import { memo, useMemo, useState } from "react";
import { ChevronDown, ChevronRight, FileCode, Terminal } from "lucide-react";
import { DiffLines, parseUnifiedDiff } from "./DiffView";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { payloadBlock } from "@/components/shared";

interface ApprovalDiffViewProps {
  toolName: string;
  args: Record<string, unknown> | null | undefined;
}

function buildDiffFromArgs(toolName: string, args: Record<string, unknown>): {
  path?: string;
  unified?: string;
  command?: string;
} | null {
  if (toolName === "terminal" && typeof args.command === "string") {
    return { command: args.command };
  }

  if (toolName === "edit") {
    const path = typeof args.path === "string" ? args.path : "";
    if (Array.isArray(args.edits) && args.edits.length > 0) {
      let combined = "";
      for (const edit of args.edits as Record<string, unknown>[]) {
        const p = typeof edit.path === "string" ? edit.path : path;
        const oldStr = typeof edit.old_string === "string" ? edit.old_string : "";
        const newStr = typeof edit.new_string === "string" ? edit.new_string : "";
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
      let combined = `--- a/${path}\n+++ b/${path}\n@@ -1,1 +1,1 @@\n`;
      for (const line of content.slice(0, 5000).split("\n")) {
        combined += `+${line}\n`;
      }
      return { path, unified: combined };
    }
  }

  return null;
}

export const ApprovalDiffView = memo(function ApprovalDiffView({
  toolName,
  args,
}: ApprovalDiffViewProps) {
  const [showRaw, setShowRaw] = useState(false);

  const parsed = useMemo(() => {
    if (!args) return null;
    return buildDiffFromArgs(toolName, args);
  }, [toolName, args]);

  const diffLines = useMemo(() => {
    if (!parsed?.unified) return null;
    return parseUnifiedDiff(parsed.unified);
  }, [parsed?.unified]);

  const rawArgs = JSON.stringify(args, null, 2) ?? "";
  const clippedArgs = rawArgs.length > 2000 ? `${rawArgs.slice(0, 2000)}…` : rawArgs;

  return (
    <div className="flex flex-col gap-2">
      {parsed?.command ? (
        <div className="rounded border border-line bg-card/60 p-2.5">
          <div className="mb-1 flex items-center gap-1.5 text-xs font-semibold text-fg-muted">
            <Terminal size={13} />
            <span>{t("実行コマンド")}</span>
          </div>
          <div className="overflow-x-auto rounded bg-inset p-2 font-mono text-xs text-brand">
            $ {parsed.command}
          </div>
        </div>
      ) : parsed?.unified && diffLines && diffLines.length > 0 ? (
        <div className="flex flex-col gap-1.5 rounded border border-line bg-card/60 p-2">
          {parsed.path && (
            <div className="flex items-center gap-1.5 px-1 text-xs font-medium text-fg-muted">
              <FileCode size={13} />
              <span className="font-mono">{parsed.path}</span>
            </div>
          )}
          <div className="max-h-60 overflow-y-auto rounded bg-inset">
            <DiffLines lines={diffLines} />
          </div>
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
          <pre className={cn(payloadBlock, "mt-1.5 max-h-40 break-all")}>
            {clippedArgs}
          </pre>
        )}
      </div>
    </div>
  );
});
