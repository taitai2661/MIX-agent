import { t } from "@/app/i18n";
import { CheckCircle2, Circle, ListTodo, Loader2 } from "lucide-react";

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
    const content = typeof value.content === "string" ? value.content.trim() : "";
    const status = value.status;
    if (!content || (status !== "pending" && status !== "in_progress" && status !== "completed")) continue;
    // `phase:<name>` markers are internal planning scaffolding, not user text.
    items.push({ content: content.replace(/^phase:\s*/i, ""), status });
  }
  return items;
}

export function TodoPane({ items }: { items?: unknown }) {
  const list = normalizeTodos(items);
  if (!list.length) return null;
  const done = list.filter((item) => item.status === "completed").length;
  return (
    <section className="todo-panel" aria-label={t("この会話のTodo")}>
      <div className="todo-panel-heading">
        <ListTodo size={15} />
        <strong>{t("Todo")}</strong>
        <span>{done}/{list.length}</span>
      </div>
      <ul className="todo-panel-list">
        {list.map((item, index) => (
          <li key={index} className={"todo-item " + item.status}>
            {item.status === "completed" ? (
              <CheckCircle2 size={15} />
            ) : item.status === "in_progress" ? (
              <Loader2 size={15} className="spin" />
            ) : (
              <Circle size={15} />
            )}
            <span className="todo-item-text">{item.content}</span>
            <span className="todo-item-status">{t(labels[item.status])}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
