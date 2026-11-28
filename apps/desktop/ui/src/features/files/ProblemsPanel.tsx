import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  CircleAlert,
  Info,
  Lightbulb,
  RefreshCw,
  TriangleAlert,
} from "lucide-react";
import { api } from "@/lib/api";
import { t, tf } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { useActiveRoot } from "@/lib/queries";
import { PanelHeader, PANEL_BODY } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { EmptyState, ErrorBox, ListSkeleton } from "@/components/shared";
import { cn } from "@/lib/cn";

type Severity = "error" | "warning" | "info" | "hint";

interface Problem {
  path: string;
  line: number | null;
  column: number | null;
  severity: Severity;
  message: string;
  source: string | null;
  code: string | null;
}

const SEVERITY_ORDER: Record<Severity, number> = {
  error: 0,
  warning: 1,
  info: 2,
  hint: 3,
};

const SEVERITY_ICON = {
  error: CircleAlert,
  warning: TriangleAlert,
  info: Info,
  hint: Lightbulb,
} as const;

const SEVERITY_TONE: Record<Severity, string> = {
  error: "text-danger",
  warning: "text-warning",
  info: "text-muted",
  hint: "text-faint",
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function asSeverity(value: unknown): Severity {
  return value === "error" || value === "warning" || value === "hint"
    ? value
    : "info";
}

/** Latest `lsp` diagnostics per file, from the conversation's tool history. */
export function extractProblems(calls: readonly unknown[]): Problem[] {
  const latest = new Map<string, Problem[]>();
  for (const call of calls) {
    if (!isRecord(call) || call.tool_name !== "lsp") continue;
    const result = call.result;
    if (!isRecord(result) || result.action !== "diagnostics") continue;
    const path = typeof result.path === "string" ? result.path : "";
    if (!path) continue;
    const items = Array.isArray(result.diagnostics) ? result.diagnostics : [];
    const parsed: Problem[] = [];
    for (const item of items) {
      if (!isRecord(item)) continue;
      const rawMessage = item.message;
      const message =
        typeof rawMessage === "string"
          ? rawMessage
          : rawMessage
            ? JSON.stringify(rawMessage)
            : "";
      if (!message) continue;
      parsed.push({
        path,
        line: typeof item.line === "number" ? item.line : null,
        column: typeof item.column === "number" ? item.column : null,
        severity: asSeverity(item.severity),
        message,
        source: typeof item.source === "string" ? item.source : null,
        code:
          typeof item.code === "string" || typeof item.code === "number"
            ? String(item.code)
            : null,
      });
    }
    // Calls arrive oldest → newest, so a later diagnostics run for the same
    // file replaces the stale one instead of piling up duplicates.
    latest.set(path, parsed);
  }
  const problems: Problem[] = [];
  for (const [path, items] of latest) {
    for (const item of items) problems.push({ ...item, path });
  }
  return problems.sort(
    (a, b) =>
      SEVERITY_ORDER[a.severity] - SEVERITY_ORDER[b.severity] ||
      a.path.localeCompare(b.path) ||
      (a.line ?? 0) - (b.line ?? 0),
  );
}

function relativePath(root: string | null, path: string): string {
  if (!root) return path;
  const normalized = root.replace(/[\\/]+$/, "");
  if (path.startsWith(`${normalized}/`))
    return path.slice(normalized.length + 1);
  if (path.startsWith(`${normalized}\\`))
    return path.slice(normalized.length + 1);
  return path;
}

export function ProblemsPanel() {
  const conversationId = useUi((s) => s.conversationId);
  const openFilePreview = useUi((s) => s.openFilePreview);
  const closeRightPanel = useUi((s) => s.closeRightPanel);
  const root = useActiveRoot();

  const history = useQuery({
    queryKey: ["conversation_tool_calls", conversationId],
    queryFn: () => api.listConversationToolCalls(conversationId!),
    enabled: !!conversationId,
    staleTime: 10_000,
  });

  const problems = useMemo(
    () => extractProblems(history.data?.calls ?? []),
    [history.data?.calls],
  );

  const byFile = useMemo(() => {
    const map = new Map<string, Problem[]>();
    for (const problem of problems) {
      const bucket = map.get(problem.path) ?? [];
      bucket.push(problem);
      map.set(problem.path, bucket);
    }
    return [...map.entries()];
  }, [problems]);

  return (
    <>
      <PanelHeader
        icon={<CircleAlert size={14} />}
        title={t("問題")}
        onClose={closeRightPanel}
        trailing={
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => void history.refetch()}
            disabled={history.isFetching || !conversationId}
            title={t("更新")}
            aria-label={t("更新")}
          >
            <RefreshCw
              size={12}
              className={history.isFetching ? "animate-spin" : undefined}
            />
          </Button>
        }
      />
      <div className={PANEL_BODY}>
        {history.isPending ? (
          <ListSkeleton rows={2} />
        ) : history.isError ? (
          <ErrorBox error={history.error} />
        ) : problems.length === 0 ? (
          <EmptyState
            title={t("診断はまだありません")}
            sub={t(
              "エージェントが lsp ツールで診断を実行すると、ここに結果が表示されます。",
            )}
          />
        ) : (
          <>
            <div className="flex items-center justify-between px-0.5 text-2xs text-faint">
              <span>{tf("{count}件の問題", { count: problems.length })}</span>
              <span>{tf("{count}ファイル", { count: byFile.length })}</span>
            </div>
            {byFile.map(([path, items]) => {
              const label = relativePath(root, path);
              return (
                <section key={path} className="flex flex-col gap-0.5">
                  <h3
                    className="truncate px-1 font-mono text-2xs font-semibold text-muted"
                    title={label}
                  >
                    {label}
                  </h3>
                  <ul className="flex flex-col gap-0.5">
                    {items.map((problem, index) => {
                      const Icon = SEVERITY_ICON[problem.severity];
                      return (
                        <li key={`${problem.line}-${index}`}>
                          <button
                            type="button"
                            onClick={() =>
                              openFilePreview(label, problem.line ?? 1)
                            }
                            className="flex w-full min-w-0 items-start gap-1.5 rounded-md px-1.5 py-1 text-start transition-colors hover:bg-hover"
                          >
                            <Icon
                              size={12}
                              className={cn(
                                "mt-0.5 shrink-0",
                                SEVERITY_TONE[problem.severity],
                              )}
                              aria-hidden="true"
                            />
                            <span className="min-w-0 flex-1">
                              <span className="block break-words text-xs text-fg">
                                {problem.message}
                              </span>
                              <span className="block font-mono text-2xs text-faint">
                                {problem.line ? `L${problem.line}` : ""}
                                {problem.column ? `:${problem.column}` : ""}
                                {problem.source ? ` · ${problem.source}` : ""}
                                {problem.code ? ` (${problem.code})` : ""}
                              </span>
                            </span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                </section>
              );
            })}
          </>
        )}
      </div>
    </>
  );
}
