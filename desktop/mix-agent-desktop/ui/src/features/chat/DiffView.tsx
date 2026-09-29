import { useQuery } from "@tanstack/react-query";
import { FileDiff, FilePlus2, X } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { useUi } from "@/lib/ui";
import { t } from "@/lib/i18n";
import { shortId } from "@/lib/format";
import { useActiveRoot } from "@/features/files/useActiveProject";
import { Alert, Skeleton } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

interface DiffLine {
  kind: "add" | "del" | "ctx" | "hunk";
  oldNo: number | null;
  newNo: number | null;
  text: string;
}

/** Parse a unified diff body into renderable lines. */
export function parseUnifiedDiff(unified: string): DiffLine[] {
  const out: DiffLine[] = [];
  let oldNo = 0;
  let newNo = 0;
  for (const line of unified.split("\n")) {
    // The trailing "" from the final newline is not a content line; genuinely
    // empty context/added/removed lines always carry their prefix character.
    if (line === "") continue;
    if (line.startsWith("--- ") || line.startsWith("+++ ")) continue;
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
    if (hunk) {
      newNo = Number(hunk[1]);
      const oldMatch = /^@@ -(\d+)(?:,\d+)? /.exec(line);
      oldNo = oldMatch ? Number(oldMatch[1]) : 0;
      out.push({ kind: "hunk", oldNo: null, newNo: null, text: line });
      continue;
    }
    if (line.startsWith("+")) {
      out.push({ kind: "add", oldNo: null, newNo, text: line.slice(1) });
      newNo += 1;
    } else if (line.startsWith("-")) {
      out.push({ kind: "del", oldNo, newNo: null, text: line.slice(1) });
      oldNo += 1;
    } else {
      out.push({ kind: "ctx", oldNo, newNo, text: line.slice(1) });
      oldNo += 1;
      newNo += 1;
    }
  }
  return out;
}

const LINE_TONE: Record<DiffLine["kind"], string> = {
  add: "bg-diff-add text-diff-add-fg",
  del: "bg-diff-del text-diff-del-fg",
  ctx: "text-fg",
  hunk: "bg-brand-soft text-brand",
};

/** Shared renderer for a parsed unified diff; used by both diff surfaces. */
export function DiffLines({ lines }: { lines: DiffLine[] }) {
  return (
    <div className="overflow-x-auto rounded-lg border border-line">
      <div className="min-w-max font-mono text-xs leading-5">
        {lines.map((line, i) => (
          <div
            key={i}
            className={cn(
              "grid grid-cols-[2.5rem_2.5rem_1fr]",
              LINE_TONE[line.kind],
              line.kind === "hunk" && "border-y border-line",
            )}
          >
            <span className="select-none border-e border-line/60 px-1.5 text-end text-faint">
              {line.oldNo ?? ""}
            </span>
            <span className="select-none border-e border-line/60 px-1.5 text-end text-faint">
              {line.newNo ?? ""}
            </span>
            <span className="whitespace-pre-wrap break-all px-2">{line.text}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function DiffView({ path, content }: { path: string; content: string }) {
  const rootPath = useActiveRoot();
  const diff = useQuery({
    queryKey: ["file_diff", path, content, rootPath],
    queryFn: () => api.previewFileDiff(path, content, rootPath),
    staleTime: 30_000,
  });

  if (diff.isPending) {
    return <Skeleton className="h-32 w-full" />;
  }
  if (diff.isError) {
    return <Alert tone="danger">{String(diff.error)}</Alert>;
  }

  const data = diff.data;
  const lines = parseUnifiedDiff(data.unified_diff);
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-1.5">
        {data.is_new
          ? <FilePlus2 size={13} className="shrink-0 text-success" />
          : <FileDiff size={13} className="shrink-0 text-faint" />}
        <span className="min-w-0 flex-1 truncate font-mono text-xs font-medium text-fg">
          {data.path}
        </span>
        {data.added > 0 && <Badge tone="success" size="sm">+{data.added}</Badge>}
        {data.removed > 0 && <Badge tone="danger" size="sm">-{data.removed}</Badge>}
        {data.is_new && <Badge tone="info" size="sm">{t("新規ファイル")}</Badge>}
      </div>

      {data.truncated && (
        <Alert tone="warning">
          {t("差分")}: {shortId(path, 24)} — {t("binary or unreadable old content")}
        </Alert>
      )}

      {lines.length === 0 ? (
        <p className="py-6 text-center text-xs text-faint">{t("差分")}: 0</p>
      ) : (
        <DiffLines lines={lines} />
      )}
    </div>
  );
}

/** Right-panel wrapper: renders the active diff target with a close button. */
export function DiffPanel() {
  const diffTarget = useUi((s) => s.diffTarget);
  const setRightPanel = useUi((s) => s.setRightPanel);
  if (!diffTarget) return null;
  return (
    <>
      <div className="flex h-9 shrink-0 items-center gap-1.5 border-b border-line px-3">
        <FileDiff size={14} className="shrink-0 text-faint" />
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-fg">{diffTarget.label}</span>
        <Button
          variant="ghost"
          size="icon-sm"
          title={t("閉じる")}
          aria-label={t("閉じる")}
          onClick={() => setRightPanel("activity")}
        >
          <X size={13} />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        <DiffView path={diffTarget.path} content={diffTarget.content} />
      </div>
    </>
  );
}
