import { memo, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { FileDiff, FilePlus2 } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { useUi } from "@/lib/ui";
import { t, tf } from "@/lib/i18n";
import { shortId } from "@/lib/format";
import { useActiveRoot } from "@/lib/queries";
import { Alert, Skeleton } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { EmptyState, ErrorAlert } from "@/components/shared";
import { PanelHeader, PANEL_BODY } from "@/components/layout";

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
  let inHunk = false;
  for (const line of unified.split("\n")) {
    // The trailing "" from the final newline is not a content line; genuinely
    // empty context/added/removed lines always carry their prefix character.
    if (line === "") continue;
    if (line.startsWith("--- ") || line.startsWith("+++ ")) continue;
    if (
      line.startsWith("diff ") ||
      line.startsWith("index ") ||
      line.startsWith("old mode") ||
      line.startsWith("new mode") ||
      line.startsWith("Binary files ") ||
      line.startsWith("\\")
    ) {
      continue;
    }
    // Tolerate `@@-1,2 +3,4@@` (no spaces) as well as the canonical form.
    const hunk = /^@@\s*-(\d+)(?:,\d+)?\s*\+(\d+)(?:,\d+)?\s*@@/.exec(line);
    if (hunk) {
      oldNo = Number(hunk[1]);
      newNo = Number(hunk[2]);
      inHunk = true;
      out.push({ kind: "hunk", oldNo: null, newNo: null, text: line });
      continue;
    }
    if (!inHunk) continue;
    if (line.startsWith("+")) {
      out.push({ kind: "add", oldNo: null, newNo, text: line.slice(1) });
      newNo += 1;
    } else if (line.startsWith("-")) {
      out.push({ kind: "del", oldNo, newNo: null, text: line.slice(1) });
      oldNo += 1;
    } else if (line.startsWith(" ")) {
      out.push({ kind: "ctx", oldNo, newNo, text: line.slice(1) });
      oldNo += 1;
      newNo += 1;
    } else {
      // Unknown trailer (e.g. a malformed header): surface it without
      // advancing either counter so line numbers below stay correct.
      out.push({ kind: "hunk", oldNo: null, newNo: null, text: line });
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

/**
 * Maximum rendered diff rows. `parseUnifiedDiff` is synchronous, so a
 * vendored/generated file with tens of thousands of lines would mount that
 * many row divs in one frame and freeze the panel. Capping keeps the parse
 * cheap enough that no virtualization or time-slicing is needed; the notice
 * below tells the user the view is truncated.
 */
const MAX_DIFF_LINES = 2000;

interface SplitDiffRow {
  isHunk: boolean;
  hunkText?: string;
  leftNo?: number | null;
  leftText?: string;
  leftKind?: "del" | "ctx" | "empty";
  rightNo?: number | null;
  rightText?: string;
  rightKind?: "add" | "ctx" | "empty";
}

export function buildSplitRows(lines: DiffLine[]): SplitDiffRow[] {
  const rows: SplitDiffRow[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.kind === "hunk") {
      rows.push({ isHunk: true, hunkText: line.text });
      i++;
      continue;
    }
    if (line.kind === "ctx") {
      rows.push({
        isHunk: false,
        leftNo: line.oldNo,
        leftText: line.text,
        leftKind: "ctx",
        rightNo: line.newNo,
        rightText: line.text,
        rightKind: "ctx",
      });
      i++;
      continue;
    }
    const dels: DiffLine[] = [];
    while (i < lines.length && lines[i].kind === "del") {
      dels.push(lines[i]);
      i++;
    }
    const adds: DiffLine[] = [];
    while (i < lines.length && lines[i].kind === "add") {
      adds.push(lines[i]);
      i++;
    }
    const count = Math.max(dels.length, adds.length);
    for (let k = 0; k < count; k++) {
      const d = dels[k];
      const a = adds[k];
      rows.push({
        isHunk: false,
        leftNo: d ? d.oldNo : null,
        leftText: d ? d.text : "",
        leftKind: d ? "del" : "empty",
        rightNo: a ? a.newNo : null,
        rightText: a ? a.text : "",
        rightKind: a ? "add" : "empty",
      });
    }
  }
  return rows;
}

/** Shared renderer for a parsed unified diff; used by both diff surfaces. */
export const DiffLines = memo(function DiffLines({
  lines,
  split = false,
  showLineNumbers = true,
}: {
  lines: DiffLine[];
  split?: boolean;
  /** Approval previews show a before/after excerpt whose absolute line
   *  numbers are unknown; rendering 1,2,3 there looked like a real location. */
  showLineNumbers?: boolean;
}) {
  const splitRows = useMemo(
    () => (split ? buildSplitRows(lines) : []),
    [lines, split],
  );

  if (split) {
    return (
      <div className="overflow-x-auto rounded-lg border border-line bg-surface">
        <div className="min-w-max font-mono text-xs leading-5">
          {splitRows.map((row, i) => {
            if (row.isHunk) {
              return (
                <div
                  key={i}
                  className="border-y border-line bg-brand-soft px-3 py-0.5 text-2xs text-brand font-semibold select-none"
                >
                  {row.hunkText}
                </div>
              );
            }
            return (
              <div
                key={i}
                className="grid grid-cols-2 divide-x divide-line border-b border-line/40 last:border-b-0"
              >
                {/* Left (Old) */}
                <div
                  className={cn(
                    "grid grid-cols-[2.5rem_1fr]",
                    row.leftKind === "del" && "bg-diff-del text-diff-del-fg",
                    row.leftKind === "empty" && "bg-inset/30",
                  )}
                >
                  <span className="select-none border-e border-line px-1.5 text-end text-faint">
                    {row.leftNo ?? ""}
                  </span>
                  <span className="whitespace-pre-wrap break-all px-2">
                    {row.leftText}
                  </span>
                </div>
                {/* Right (New) */}
                <div
                  className={cn(
                    "grid grid-cols-[2.5rem_1fr]",
                    row.rightKind === "add" && "bg-diff-add text-diff-add-fg",
                    row.rightKind === "empty" && "bg-inset/30",
                  )}
                >
                  <span className="select-none border-e border-line px-1.5 text-end text-faint">
                    {row.rightNo ?? ""}
                  </span>
                  <span className="whitespace-pre-wrap break-all px-2">
                    {row.rightText}
                  </span>
                </div>
              </div>
            );
          })}
        </div>
      </div>
    );
  }

  return (
    <div className="overflow-x-auto rounded-lg border border-line">
      <div className="min-w-max font-mono text-xs leading-5">
        {lines.map((line, i) =>
          line.kind === "hunk" && !showLineNumbers ? null : (
            <div
              key={i}
              className={cn(
                "grid",
                showLineNumbers
                  ? "grid-cols-[2.5rem_2.5rem_1fr]"
                  : "grid-cols-[1fr]",
                LINE_TONE[line.kind],
                line.kind === "hunk" && "border-y border-line",
              )}
            >
              {showLineNumbers && (
                <>
                  <span className="select-none border-e border-line px-1.5 text-end text-faint">
                    {line.oldNo ?? ""}
                  </span>
                  <span className="select-none border-e border-line px-1.5 text-end text-faint">
                    {line.newNo ?? ""}
                  </span>
                </>
              )}
              <span className="whitespace-pre-wrap break-all px-2">
                {line.text}
              </span>
            </div>
          ),
        )}
      </div>
    </div>
  );
});

export function DiffView({
  path,
  content,
  oldString,
  newString,
}: {
  path: string;
  content: string;
  oldString?: string;
  newString?: string;
}) {
  const rootPath = useActiveRoot();
  const diff = useQuery({
    queryKey: ["file_diff", path, content, rootPath, oldString, newString],
    queryFn: () =>
      api.previewFileDiff(path, content, rootPath, oldString, newString),
    staleTime: 30_000,
  });

  const data = diff.data;
  // Parsing runs per render today; memoise it and cap huge diffs so a vendored
  // file cannot mount tens of thousands of row divs at once (see
  // `MAX_DIFF_LINES`). The hook stays above every early return so hook order
  // never changes between renders. `capped` compares against the pre-slice
  // length: exactly `MAX_DIFF_LINES` lines is not truncated and must not show
  // the notice.
  const { lines, capped } = useMemo(() => {
    if (!data)
      return {
        lines: [] as ReturnType<typeof parseUnifiedDiff>,
        capped: false,
      };
    const parsed = parseUnifiedDiff(data.unified_diff);
    return {
      lines: parsed.slice(0, MAX_DIFF_LINES),
      capped: parsed.length > MAX_DIFF_LINES,
    };
  }, [data]);

  if (diff.isPending) {
    return <Skeleton className="h-32 w-full" />;
  }
  if (diff.isError) {
    return <ErrorAlert error={diff.error} />;
  }
  if (!data) {
    return <Skeleton className="h-32 w-full" />;
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-1.5">
        {data.is_new ? (
          <FilePlus2 size={13} className="shrink-0 text-success" />
        ) : (
          <FileDiff size={13} className="shrink-0 text-faint" />
        )}
        <span className="min-w-0 flex-1 truncate font-mono text-xs font-medium text-fg">
          {data.path}
        </span>
        {data.added > 0 && (
          <Badge tone="success" size="sm">
            +{data.added}
          </Badge>
        )}
        {data.removed > 0 && (
          <Badge tone="danger" size="sm">
            -{data.removed}
          </Badge>
        )}
        {data.is_new && (
          <Badge tone="info" size="sm">
            {t("新規ファイル")}
          </Badge>
        )}
      </div>

      {data.truncated && (
        <Alert tone="warning">
          {t("差分")}: {shortId(path, 24)} —{" "}
          {t("バイナリまたは読み取れない旧内容")}
        </Alert>
      )}

      {lines.length === 0 ? (
        <EmptyState compact title={tf("差分: {n}", { n: 0 })} />
      ) : (
        <>
          <DiffLines lines={lines} />
          {capped && (
            <p className="text-xs text-faint">
              {t("大きな差分のため先頭2000行のみ表示します")}
            </p>
          )}
        </>
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
      <PanelHeader
        icon={<FileDiff size={14} />}
        title={diffTarget.label}
        onClose={() => setRightPanel("none")}
      />
      <div className={PANEL_BODY}>
        <DiffView
          path={diffTarget.path}
          content={diffTarget.content}
          oldString={diffTarget.oldString}
          newString={diffTarget.newString}
        />
      </div>
    </>
  );
}
