import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { useQueryClient } from "@tanstack/react-query";
import { Brain } from "lucide-react";
import { useState } from "react";

import { ErrorBox, Field, Title, useRows } from "@/components/shared";

export function Memories() {
  const rows = useRows("/memories"),
    qc = useQueryClient();
  const [q, setQ] = useState(""),
    [editing, setEditing] = useState<Row | null>(null),
    [content, setContent] = useState(""),
    [error, setError] = useState<unknown>(null),
    [history, setHistory] = useState<{ memory: string; rows: Row[] } | null>(
      null,
    );
  return (
    <main className="page">
      <Title
        title="Memory"
        sub="好みや決定を、次の会話へ。追加・更新は自動、削除は承認制です。"
      />
      <ErrorBox error={error || rows.error} />
      <form
        className="card"
        onSubmit={async (e) => {
          e.preventDefault();
          try {
            await api(
              "/memories" + (editing ? "/" + editing.id : ""),
              editing ? "PATCH" : "POST",
              { content },
            );
            setContent("");
            setEditing(null);
            qc.invalidateQueries({ queryKey: ["/memories"] });
          } catch (e) {
            setError(e);
          }
        }}
      >
        <Field label={editing ? "Memoryを編集" : "新しいMemory"}>
          <textarea
            value={content}
            onChange={(e) => setContent(e.target.value)}
            required
            placeholder="例：回答は日本語で、簡潔な説明を好む"
          />
        </Field>
        <div className="form-actions">
          <Button>{editing ? "更新" : "保存"}</Button>
          {editing && (
            <Button
              type="button"
              variant="ghost"
              onClick={() => {
                setEditing(null);
                setContent("");
              }}
            >
              キャンセル
            </Button>
          )}
        </div>
      </form>
      <input
        className="search-input"
        aria-label="Memory検索"
        placeholder="Memoryを検索…"
        value={q}
        onChange={(e) => setQ(e.target.value)}
      />
      {rows.data
        ?.filter((r) => r.data.content.includes(q))
        .map((r) => (
          <div
            className={"card memory-card " + (r.data.deleted ? "deleted" : "")}
            key={r.id}
          >
            <Brain size={20} />
            <div className="grow">
              <p>{r.data.content}</p>
              <small>
                {r.data.scope} ·{" "}
                {r.data.source_run ? "Agentが保存" : "手動で保存"}
                {r.data.deleted ? " · 削除済み" : ""}
              </small>
            </div>
            <div className="row-actions">
              {!r.data.deleted && (
                <Button
                  variant="ghost"
                  onClick={() => {
                    setEditing(r);
                    setContent(r.data.content);
                  }}
                >
                  編集
                </Button>
              )}
              <Button
                variant="ghost"
                onClick={async () => {
                  try {
                    setHistory({
                      memory: r.id,
                      rows: await api("/memories/" + r.id + "/revisions"),
                    });
                  } catch (e) {
                    setError(e);
                  }
                }}
              >
                履歴
              </Button>
              {!r.data.deleted && (
                <Button
                  variant="ghost"
                  onClick={async () => {
                    if (
                      confirm(
                        "このMemoryを削除しますか？履歴から復元できます。",
                      )
                    ) {
                      try {
                        await api("/memories/" + r.id, "DELETE");
                        qc.invalidateQueries({ queryKey: ["/memories"] });
                      } catch (e) {
                        setError(e);
                      }
                    }
                  }}
                >
                  削除
                </Button>
              )}
            </div>
          </div>
        ))}
      {history && (
        <div className="card">
          <h3>変更履歴</h3>
          {history.rows.map((r) => (
            <div className="revision" key={r.id}>
              <p>{r.data.previous.content}</p>
              <Button
                variant="outline"
                onClick={async () => {
                  try {
                    await api(
                      "/memories/" + history.memory + "/restore/" + r.id,
                      "POST",
                    );
                    qc.invalidateQueries({ queryKey: ["/memories"] });
                    setHistory(null);
                  } catch (e) {
                    setError(e);
                  }
                }}
              >
                復元
              </Button>
            </div>
          ))}
          {!history.rows.length && <p>変更履歴はまだありません。</p>}
          <Button variant="ghost" onClick={() => setHistory(null)}>
            閉じる
          </Button>
        </div>
      )}
    </main>
  );
}
