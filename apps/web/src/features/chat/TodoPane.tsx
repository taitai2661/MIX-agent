import { t } from "@/app/i18n";
import { useState } from "react";
import {
  CheckCircle2,
  ChevronDown,
  Circle,
  ListTodo,
  Loader2,
} from "lucide-react";

type Status = "pending" | "in_progress" | "completed";
type TodoItem = { content: string; status: Status };

const labels: Record<Status, string> = {
  completed: "完了",
  in_progress: "進行中",
  pending: "未着手",
};

/** Accepts the loosely typed server payload and drops malformed entries. */
export function normalizeTodos(raw: unknown): TodoItem[] {
  if (!Array.isArray(raw)) return [];
  const items: TodoItem[] = [];
  for (const entry of raw) {
    if (!entry || typeof entry !== "object") continue;
    const value = entry as { content?: unknown; status?: unknown };
    const content =
      typeof value.content === "string" ? value.content.trim() : "";
    const status = value.status;
    if (
      !content ||
      (status !== "pending" &&
        status !== "in_progress" &&
        status !== "completed")
    )
      continue;
    // `phase:<name>` markers are internal planning scaffolding, not user text.
    items.push({ content: content.replace(/^phase:\s*/i, ""), status });
  }
  return items;
}

export function TodoPane({ items }: { items?: unknown }) {
  const [userOpen, setUserOpen] = useState<boolean | null>(null);
  const list = normalizeTodos(items);
  if (!list.length) return null;
  const done = list.filter((item) => item.status === "completed").length;
  const allCompleted = done === list.length;
  const current =
    list.find((item) => item.status === "in_progress") ??
    list.find((item) => item.status === "pending");
  const progressPercent = Math.round((done / list.length) * 100);
  const detailsOpen = userOpen ?? !allCompleted;

  return (
    <section
      className={`todo-panel${allCompleted ? " is-completed" : ""}`}
      aria-label={t("この会話のTodo")}
    >
      <details
        className="todo-panel-details"
        open={detailsOpen}
        onToggle={(e) => setUserOpen(e.currentTarget.open)}
      >
        <summary className="todo-panel-heading">
          <div className="todo-panel-summary-left">
            <span className="todo-panel-icon">
              {allCompleted ? (
                <CheckCircle2 size={15} />
              ) : (
                <ListTodo size={15} />
              )}
            </span>
            <strong className="todo-panel-title">{t("Todo")}</strong>
            <span className="todo-panel-badge">
              {done}/{list.length}
            </span>
            {current && (
              <span className="todo-panel-current" title={current.content}>
                {current.content}
              </span>
            )}
          </div>
          <ChevronDown size={14} className="todo-panel-chevron" />
        </summary>
        <div className="todo-panel-progress-track">
          <div
            className="todo-panel-progress-bar"
            style={{ width: `${progressPercent}%` }}
          />
        </div>
        <ul className="todo-panel-list">
          {list.map((item, index) => (
            <li key={index} className={"todo-item " + item.status}>
              <span className="todo-item-icon">
                {item.status === "completed" ? (
                  <CheckCircle2 size={15} />
                ) : item.status === "in_progress" ? (
                  <Loader2 size={15} className="spin" />
                ) : (
                  <Circle size={15} />
                )}
              </span>
              <span className="todo-item-text">{item.content}</span>
              <span className={"todo-item-status " + item.status}>
                {t(labels[item.status])}
              </span>
            </li>
          ))}
        </ul>
      </details>
    </section>
  );
}
