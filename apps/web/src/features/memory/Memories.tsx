import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Field, Title, useRows } from "@/components/shared";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Brain, GitBranch } from "lucide-react";
import { useState } from "react";

const states: Record<string, string> = { latent: "未定着", established: "定着", superseded: "過去の状態", archived: "アーカイブ", deleted: "削除済み" };
const percent = (value: unknown) => `${Math.round(Number(value || 0) * 100)}%`;

export function Memories() {
  const qc = useQueryClient();
  const [q, setQ] = useState(""), [state, setState] = useState(""), [editing, setEditing] = useState<Row | null>(null),
    [content, setContent] = useState(""), [strength, setStrength] = useState(.85), [confidence, setConfidence] = useState(.95),
    [salience, setSalience] = useState(.7), [error, setError] = useState<unknown>(null),
    [history, setHistory] = useState<{ memory: string; rows: Row[] } | null>(null),
    [relations, setRelations] = useState<{ memory: string; rows: any[] } | null>(null);
  const rows = useRows(`/memories?q=${encodeURIComponent(q)}${state ? `&state=${state}` : ""}`);
  const settings = useQuery<any>({ queryKey: ["/settings"], queryFn: () => api("/settings") });
  const refreshMemories = () => qc.invalidateQueries({ predicate: item => String(item.queryKey[0]).startsWith("/memories") });
  const reset = () => { setEditing(null); setContent(""); setStrength(.85); setConfidence(.95); setSalience(.7); };
  return <main className="page">
    <Title title="Associative Memory" sub="記憶を分類箱ではなく、文脈で意味が変わるTraceネットワークとして管理します。" />
    <ErrorBox error={error || rows.error || settings.error} />
    {settings.data && <form className="card memory-settings" onSubmit={async e => {
      e.preventDefault(); const form = new FormData(e.currentTarget);
      try { const { has_secret_id, has_brave_secret_id, has_tavily_secret_id, has_exa_secret_id, has_serper_secret_id, ...rest } = settings.data.data;
        await api("/settings", "PUT", { ...rest,
        memory_auto_formation: form.get("auto") === "on", memory_max_depth: Number(form.get("depth")),
        memory_max_candidates: Number(form.get("candidates")), memory_retrieval_budget_ms: Number(form.get("budget")),
        memory_min_association_weight: Number(form.get("weight")), memory_activation_decay: Number(form.get("decay")), brave_api_key: null, tavily_api_key: null, exa_api_key: null, serper_api_key: null,
      }); await qc.invalidateQueries({ queryKey: ["/settings"] }); } catch (reason) { setError(reason); }
    }}>
      <h3>形成と想起</h3>
      <label className="check"><input name="auto" type="checkbox" defaultChecked={settings.data.data.memory_auto_formation !== false} />回答後に候補Traceを非同期形成</label>
      <div className="memory-setting-grid">
        <Field label="最大hop"><input name="depth" type="number" min="0" max="3" defaultValue={settings.data.data.memory_max_depth ?? 2} /></Field>
        <Field label="最大候補"><input name="candidates" type="number" min="8" max="256" defaultValue={settings.data.data.memory_max_candidates ?? 96} /></Field>
        <Field label="検索予算(ms)"><input name="budget" type="number" min="20" max="1000" defaultValue={settings.data.data.memory_retrieval_budget_ms ?? 120} /></Field>
        <Field label="最小関連weight"><input name="weight" type="number" min=".05" max="1" step=".05" defaultValue={settings.data.data.memory_min_association_weight ?? .2} /></Field>
        <Field label="活性化decay"><input name="decay" type="number" min=".1" max=".95" step=".05" defaultValue={settings.data.data.memory_activation_decay ?? .55} /></Field>
      </div><Button>設定を保存</Button>
    </form>}
    <form className="card" onSubmit={async e => { e.preventDefault(); try {
      await api(`/memories${editing ? `/${editing.id}` : ""}`, editing ? "PATCH" : "POST", { content, strength, confidence, salience, lifecycle_state: "established" });
      reset(); refreshMemories();
    } catch (reason) { setError(reason); } }}>
      <Field label={editing ? "Traceを訂正" : "明示的なTraceを追加"}><textarea required value={content} onChange={e => setContent(e.target.value)} placeholder="例：回答は日本語で、簡潔な説明を好む" /></Field>
      <div className="memory-setting-grid">
        <Field label={`Strength ${percent(strength)}`}><input type="range" min="0" max="1" step=".05" value={strength} onChange={e => setStrength(Number(e.target.value))} /></Field>
        <Field label={`Confidence ${percent(confidence)}`}><input type="range" min="0" max="1" step=".05" value={confidence} onChange={e => setConfidence(Number(e.target.value))} /></Field>
        <Field label={`Salience ${percent(salience)}`}><input type="range" min="0" max="1" step=".05" value={salience} onChange={e => setSalience(Number(e.target.value))} /></Field>
      </div><div className="form-actions"><Button>{editing ? "訂正" : "追加"}</Button>{editing && <Button type="button" variant="ghost" onClick={reset}>キャンセル</Button>}</div>
    </form>
    <div className="memory-filters"><input className="search-input" aria-label="Memory検索" placeholder="Traceを検索…" value={q} onChange={e => setQ(e.target.value)} />
      <select aria-label="定着状態" value={state} onChange={e => setState(e.target.value)}><option value="">すべての状態</option>{Object.entries(states).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></div>
    {rows.data?.map(row => <div className={`card memory-card ${row.data.lifecycle_state === "deleted" ? "deleted" : ""}`} key={row.id}>
      <Brain size={20} /><div className="grow"><p>{row.data.content}</p><div className="memory-metrics"><span>{states[row.data.lifecycle_state] || row.data.lifecycle_state}</span><span>強度 {percent(row.data.strength)}</span><span>確度 {percent(row.data.confidence)}</span><span>顕著性 {percent(row.data.salience)}</span><span>活性化 {row.data.activation_count || 0}回</span></div>
      {!!row.data.concepts?.length && <small>概念: {row.data.concepts.join("、")}</small>}</div>
      <div className="row-actions">{row.data.lifecycle_state !== "deleted" && <Button variant="ghost" onClick={() => { setEditing(row); setContent(row.data.content); setStrength(row.data.strength); setConfidence(row.data.confidence); setSalience(row.data.salience); }}>訂正</Button>}
        <Button variant="ghost" onClick={async () => { try { setRelations({ memory: row.id, rows: await api(`/memories/${row.id}/associations`) }); } catch (reason) { setError(reason); } }}><GitBranch size={14} /> 関連</Button>
        <Button variant="ghost" onClick={async () => { try { setHistory({ memory: row.id, rows: await api(`/memories/${row.id}/revisions`) }); } catch (reason) { setError(reason); } }}>履歴</Button>
        {row.data.lifecycle_state !== "deleted" && <Button variant="ghost" onClick={async () => { if (confirm("このTraceと派生記憶を想起対象外にしますか？履歴から復元できます。")) { try { await api(`/memories/${row.id}`, "DELETE"); refreshMemories(); } catch (reason) { setError(reason); } } }}>忘れる</Button>}
      </div></div>)}
    {relations && <div className="card"><h3>関連Trace</h3>{relations.rows.map(row => <div className="revision" key={row.id}><span>{row.relation || "association"}</span><small>weight {percent(row.weight)} · confidence {percent(row.confidence)}</small></div>)}{!relations.rows.length && <p>関連Traceはまだありません。</p>}<Button variant="ghost" onClick={() => setRelations(null)}>閉じる</Button></div>}
    {history && <div className="card"><h3>変更履歴</h3>{history.rows.map(row => <div className="revision" key={row.id}><p>{row.data.previous.content}</p><Button variant="outline" onClick={async () => { try { await api(`/memories/${history.memory}/restore/${row.id}`, "POST"); refreshMemories(); setHistory(null); } catch (reason) { setError(reason); } }}>復元</Button></div>)}{!history.rows.length && <p>変更履歴はまだありません。</p>}<Button variant="ghost" onClick={() => setHistory(null)}>閉じる</Button></div>}
  </main>;
}
