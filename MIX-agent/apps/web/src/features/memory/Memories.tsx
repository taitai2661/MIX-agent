import { t } from "@/app/i18n";
import { api } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Field, Title } from "@/components/shared";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Brain, ListChecks, RefreshCw, Search, Trash2 } from "lucide-react";
import { useState } from "react";

type MemoryRow = {
  id: string;
  data: { content?: string; gist?: string; pinned?: boolean; source?: string };
  created_at: string;
};

type ProbeHit = { id: string; content?: string; gist?: string; relevance?: number; selection_reason?: string };

function contentOf(row: MemoryRow): string {
  return row.data?.content || row.data?.gist || "";
}

export function Memories() {
  const qc = useQueryClient();
  const [q, setQ] = useState("");
  const [probeQuery, setProbeQuery] = useState("");
  const [probe, setProbe] = useState<{ memories: ProbeHit[] } | null>(null);
  const [editing, setEditing] = useState<MemoryRow | null>(null);
  const [rememberOpen, setRememberOpen] = useState(false);

  const list = useQuery<MemoryRow[]>({
    queryKey: ["/memories", q],
    queryFn: () => api(`/memories?q=${encodeURIComponent(q)}`),
  });
  const settings = useQuery<any>({ queryKey: ["/settings"], queryFn: () => api("/settings") });

  const refresh = () => {
    qc.invalidateQueries({ queryKey: ["/memories"] });
  };

  const runProbe = async (raw: string) => {
    const query = raw.trim();
    if (!query) return;
    try {
      const result = await api<{ memories: ProbeHit[] }>(`/memories-debug/search?q=${encodeURIComponent(query)}`);
      setProbe(result);
    } catch {
      setProbe({ memories: [] });
    }
  };

  const removeOne = async (id: string) => {
    await api(`/memories/${id}`, "DELETE");
    refresh();
  };

  return (
    <main className="page memory-mind-view">
      <Title title="記憶" sub={t("ChatGPTのように、ユーザーについての短い事実を1件ずつ保存します。")} />

      <ErrorBox error={list.error || settings.error} />

      {settings.data && (
        <form className="card memory-settings" onSubmit={async (e) => {
          e.preventDefault();
          const form = new FormData(e.currentTarget);
          try {
            const { has_secret_id, has_brave_secret_id, has_tavily_secret_id, has_exa_secret_id, has_serper_secret_id, ...rest } = settings.data.data;
            await api("/settings", "PUT", {
              ...rest,
              memory_auto_formation: form.get("auto") === "on",
              brave_api_key: null,
              tavily_api_key: null,
              exa_api_key: null,
              serper_api_key: null,
            });
            await qc.invalidateQueries({ queryKey: ["/settings"] });
          } catch (reason) {
            console.error(reason);
          }
        }}>
          <h3>{t("自動記憶")}</h3>
          <label className="check">
            <input name="auto" type="checkbox" defaultChecked={settings.data.data.memory_auto_formation !== false} />
            {t("回答後にユーザー facts を自動抽出する")}
          </label>
          <Button>{t("設定を保存")}</Button>
        </form>
      )}

      <section className="card memory-search-section">
        <div className="memory-toolbar">
          <div className="memory-toolbar-left">
            <Search size={16} />
            <input
              className="search-input"
              aria-label={t("記憶を検索")}
              placeholder={t("例: 日本語 / Postgres")}
              value={q}
              onChange={(e) => setQ(e.target.value)}
            />
            <Button onClick={refresh}><RefreshCw size={14} /> {t("更新")}</Button>
          </div>
          <div className="memory-toolbar-right">
            <Button onClick={() => setRememberOpen(true)}>
              <ListChecks size={14} /> {t("覚える")}
            </Button>
          </div>
        </div>
      </section>

      <section className="card memory-mind-section">
        <header>
          <h3><Brain size={16} /> {t("保存された記憶")} <small>{list.data?.length ?? 0}</small></h3>
        </header>
        {!list.data?.length ? (
          <p>{t("まだ記憶がありません。「覚える」から追加するか、「○○を覚えて」と話しかけてください。")}</p>
        ) : (
          <ul className="memory-grid">
            {list.data.map((mem) => (
              <li key={mem.id} className="card memory-card">
                <p className="memory-content">{contentOf(mem)}</p>
                <div className="memory-metrics">
                  {mem.data?.pinned ? <span>{t("固定")}</span> : null}
                  {mem.data?.source ? <span>{mem.data.source}</span> : null}
                </div>
                <div className="form-actions">
                  <Button variant="ghost" onClick={() => setEditing(mem)}>{t("編集")}</Button>
                  <Button variant="ghost" onClick={() => removeOne(mem.id)}><Trash2 size={14} /> {t("忘れる")}</Button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      {editing && (
        <EditForm
          row={editing}
          onClose={() => setEditing(null)}
          onSaved={() => { setEditing(null); refresh(); }}
        />
      )}

      {rememberOpen && (
        <RememberForm
          onClose={() => setRememberOpen(false)}
          onSaved={() => { setRememberOpen(false); refresh(); }}
        />
      )}

      <section className="card memory-debug">
        <header><h3>{t("想起プローブ")}</h3></header>
        <form onSubmit={async (e) => { e.preventDefault(); await runProbe(probeQuery); }}>
          <Field label={t("クエリを入力して一致を確認")}>
            <input required value={probeQuery} onChange={(e) => setProbeQuery(e.target.value)} placeholder={t("例: 日本語")} />
          </Field>
          <div className="form-actions"><Button>{t("想起を実行")}</Button></div>
        </form>
        {probe && (
          <ul>
            {probe.memories.map((m) => <li key={m.id}>{m.content || m.gist} {typeof m.relevance === "number" ? `(${m.relevance.toFixed(2)})` : ""}</li>)}
          </ul>
        )}
      </section>
    </main>
  );
}

function RememberForm({ onClose, onSaved }: { onClose: () => void; onSaved: () => void }) {
  const [content, setContent] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [saving, setSaving] = useState(false);
  return (
    <div className="card memory-remember-form">
      <header><h3><ListChecks size={16} /> {t("記憶を追加")}</h3></header>
      <p className="memory-auto-note">{t("短い事実を1件ずつ保存します（200字以内推奨）。重複は自動で無視されます。")}</p>
      {error ? <ErrorBox error={error} /> : null}
      <Field label={t("内容")}>
        <textarea required value={content} onChange={(e) => setContent(e.target.value)} rows={3} maxLength={1000} />
      </Field>
      <div className="form-actions">
        <Button onClick={async () => {
          setSaving(true);
          try {
            await api("/memories", "POST", { content });
            onSaved();
          } catch (reason) {
            setError(reason);
          } finally {
            setSaving(false);
          }
        }} disabled={saving || !content.trim()}>{t("保存")}</Button>
        <Button variant="ghost" onClick={onClose}>{t("キャンセル")}</Button>
      </div>
    </div>
  );
}

function EditForm({ row, onClose, onSaved }: { row: MemoryRow; onClose: () => void; onSaved: () => void }) {
  const [content, setContent] = useState(contentOf(row));
  const [error, setError] = useState<unknown>(null);
  const [saving, setSaving] = useState(false);
  return (
    <div className="card memory-remember-form">
      <header><h3>{t("記憶を編集")}</h3></header>
      {error ? <ErrorBox error={error} /> : null}
      <Field label={t("内容")}>
        <textarea required value={content} onChange={(e) => setContent(e.target.value)} rows={3} maxLength={1000} />
      </Field>
      <div className="form-actions">
        <Button onClick={async () => {
          setSaving(true);
          try {
            await api(`/memories/${row.id}`, "PATCH", { content });
            onSaved();
          } catch (reason) {
            setError(reason);
          } finally {
            setSaving(false);
          }
        }} disabled={saving || !content.trim()}>{t("保存")}</Button>
        <Button variant="ghost" onClick={onClose}>{t("キャンセル")}</Button>
      </div>
    </div>
  );
}
