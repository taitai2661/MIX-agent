import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Title } from "@/components/shared";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

type Job = Row & { data: { name: string; target_type: "agent" | "conversation"; target_id: string; prompt: string; cron: string; timezone: string; enabled: boolean; catch_up: boolean; next_at?: string | null } };
export function Schedules() {
  const qc = useQueryClient(); const [error, setError] = useState<unknown>(null);
  const [selected, setSelected] = useState("");
  const [form, setForm] = useState({ name: "", target_type: "agent", target_id: "", prompt: "", cron: "0 9 * * *", timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || "Asia/Tokyo", enabled: true, catch_up: false });
  const jobs = useQuery<Job[]>({ queryKey: ["/scheduled-jobs"], queryFn: () => api("/scheduled-jobs") });
  const agents = useQuery<Row[]>({ queryKey: ["/agents"], queryFn: () => api("/agents") });
  const conversations = useQuery<Row[]>({ queryKey: ["/conversations"], queryFn: () => api("/conversations?state=active") });
  const notifications = useQuery<Row[]>({ queryKey: ["/notifications"], queryFn: () => api("/notifications?unread=true") });
  const runs = useQuery<any[]>({ queryKey: ["/scheduled-jobs", selected, "runs"], queryFn: () => api(`/scheduled-jobs/${selected}/runs`), enabled: !!selected });
  const refresh = () => qc.invalidateQueries({ queryKey: ["/scheduled-jobs"] });
  const targets = form.target_type === "agent" ? agents.data || [] : conversations.data || [];
  const preset = ({"0 * * * *":"hourly", "0 9 * * *":"daily", "0 9 * * 1":"weekly"} as Record<string, string>)[form.cron] || "custom";
  return <main className="page"><Title title="定期実行" sub="Cron式またはフォーム相当の定期設定で、Agentと会話を自動実行します。" />
    {error !== null && <ErrorBox error={error} />}<form className="stack" onSubmit={async e => { e.preventDefault(); try { await api("/scheduled-jobs", "POST", form); setForm({ ...form, name: "", prompt: "", target_id: "" }); refresh(); } catch (x) { setError(x); } }}>
      <input required placeholder="ジョブ名" value={form.name} onChange={e => setForm({...form, name:e.target.value})} />
      <select value={form.target_type} onChange={e => setForm({...form, target_type:e.target.value, target_id:""})}><option value="agent">保存済みAgentを独立実行</option><option value="conversation">既存会話へ投稿</option></select>
      <select required value={form.target_id} onChange={e => setForm({...form, target_id:e.target.value})}><option value="">対象を選択</option>{targets.map(t => <option key={t.id} value={t.id}>{t.data.name || t.data.title}</option>)}</select>
      <textarea required placeholder="毎回送るプロンプト" value={form.prompt} onChange={e => setForm({...form, prompt:e.target.value})} />
      <label>頻度<select value={preset} onChange={e => { const cron = ({hourly:"0 * * * *", daily:"0 9 * * *", weekly:"0 9 * * 1"} as Record<string,string>)[e.target.value]; if (cron) setForm({...form, cron}); }}><option value="hourly">毎時</option><option value="daily">毎日 09:00</option><option value="weekly">毎週月曜 09:00</option><option value="custom">Cron式を直接指定</option></select></label>
      <label>Cron式（分 時 日 月 曜日）<input required value={form.cron} onChange={e => setForm({...form, cron:e.target.value})} /></label>
      <label>タイムゾーン<input required value={form.timezone} onChange={e => setForm({...form, timezone:e.target.value})} /></label>
      <label><input type="checkbox" checked={form.catch_up} onChange={e => setForm({...form, catch_up:e.target.checked})} /> 停止中の最新1回を復旧時に実行</label><Button type="submit">定期実行を作成</Button>
    </form>
    {!!notifications.data?.length && <section className="conversation-list"><h3>未読通知</h3>{notifications.data.map(n => <article className="conversation-row" key={n.id}><span>{n.data.title}</span><Button variant="outline" onClick={async()=>{await api(`/notifications/${n.id}`, "PATCH", {read:true});qc.invalidateQueries({queryKey:["/notifications"]});}}>既読</Button></article>)}</section>}
    <div className="conversation-list">{jobs.data?.map(job => <article className="conversation-row" key={job.id}><div><b>{job.data.name}</b><small>{job.data.cron} · {job.data.timezone} · 次回: {job.data.next_at ? new Date(job.data.next_at).toLocaleString() : "停止中"}</small></div><div className="row-actions"><Button variant="outline" onClick={()=>setSelected(job.id)}>履歴</Button><Button variant="outline" onClick={async()=>{await api(`/scheduled-jobs/${job.id}/run`, "POST"); refresh();}}>今すぐ実行</Button><Button variant="outline" onClick={async()=>{await api(`/scheduled-jobs/${job.id}`, "PATCH", {...job.data, enabled:!job.data.enabled}); refresh();}}>{job.data.enabled ? "停止" : "有効化"}</Button><button className="icon danger" onClick={async()=>{if(confirm("定期実行を削除しますか？")){await api(`/scheduled-jobs/${job.id}`, "DELETE");refresh();}}}>×</button></div></article>)}{!jobs.data?.length && <p className="empty">定期実行はまだありません。</p>}</div>
    {selected && <section className="conversation-list"><h3>実行履歴</h3>{runs.data?.map(run => <article className="conversation-row" key={run.id}><div><b>{run.status}</b><small>{new Date(run.scheduled_at).toLocaleString()} {run.data.reason ? `· ${run.data.reason}` : ""}</small>{run.answer && <p>{run.answer}</p>}</div></article>)}{!runs.data?.length && <p className="empty">実行履歴はまだありません。</p>}</section>}
  </main>;
}
