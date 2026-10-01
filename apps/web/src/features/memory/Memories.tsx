import { t } from "@/app/i18n";
import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Field, Title, useRows } from "@/components/shared";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  AlertTriangle,
  Brain,
  CheckCircle2,
  Circle,
  Eye,
  GitBranch,
  History,
  ListChecks,
  Network,
  RefreshCw,
  Search,
  Trash2,
  Workflow,
} from "lucide-react";
import { useMemo, useState, type ReactElement } from "react";

const ROLE_LABEL: Record<string, string> = {
  fact: "事実",
  decision: "Decision",
  preference: "Preference",
  goal: "Goal",
  constraint: "Constraint",
  experience: "Experience",
  failure: "Failure",
  solution: "Solution",
  observation: "Observation",
  hypothesis: "Hypothesis",
  question: "Pending Question",
};
const SCOPE_LABEL: Record<string, string> = {
  working: "Working",
  task: "Task",
  project: "Project",
  user: "User",
  world: "World",
};
const LIFECYCLE_LABEL: Record<string, string> = {
  candidate: "候補",
  active: "Active",
  superseded: "Superseded",
  expired: "Expired",
  archived: "Archived",
  disputed: "Disputed",
  established: "Active",
  latent: "候補",
  deleted: "Expired",
};
const ROLE_ICON: Record<string, ReactElement> = {
  decision: <CheckCircle2 size={14} />,
  failure: <AlertTriangle size={14} />,
  preference: <Circle size={14} />,
  goal: <Workflow size={14} />,
  experience: <History size={14} />,
};

const percent = (value: unknown) => `${Math.round(Number(value || 0) * 100)}%`;
type MemoryView = {
  id: string;
  content: string;
  gist?: string;
  role: string;
  scope: string;
  lifecycle: string;
  lifecycle_label: string;
  verification: string;
  confidence: number;
  salience: number;
  strength: number;
  entities?: string[];
  concepts?: string[];
  task_id?: string | null;
  superseded_by_id?: string | null;
  disputed_by_id?: string | null;
  selection_reason?: string;
  relevance?: number;
  role_metadata?: Record<string, unknown> | null;
  evidence?: Array<{ id: string; kind: string; ref?: string; summary?: string; captured_at?: string }>;
  evidence_count?: number;
};
type GroupResponse = {
  groups: Record<string, MemoryView[]>;
  totals: Record<string, number>;
};
type ConflictView = {
  id: string;
  reason: string;
  resolved: boolean;
  resolution: string;
  a: MemoryView | null;
  b: MemoryView | null;
};

function RoleBadge({ role }: { role: string }) {
  return (
    <span className={`memory-role-badge memory-role-${role}`}>
      {ROLE_ICON[role]}
      <span>{ROLE_LABEL[role] || role}</span>
    </span>
  );
}

function MemoryCard({ memory, onSelect, onConflict }: { memory: MemoryView; onSelect: (m: MemoryView) => void; onConflict?: (a: MemoryView, b: MemoryView) => void }) {
  return (
    <div className="card memory-card" data-role={memory.role} onClick={() => onSelect(memory)}>
      <div className="memory-card-header">
        <RoleBadge role={memory.role} />
        <span className="memory-scope">{SCOPE_LABEL[memory.scope] || memory.scope}</span>
        <span className="memory-lifecycle">{LIFECYCLE_LABEL[memory.lifecycle] || memory.lifecycle}</span>
        {memory.task_id && <span className="memory-task-pill">Task</span>}
        {memory.superseded_by_id && <span className="memory-superseded">→ superseded</span>}
        {memory.disputed_by_id && <span className="memory-disputed">⚠ disputed</span>}
      </div>
      <p className="memory-content">{memory.content || memory.gist}</p>
      {memory.role_metadata && memory.role === "decision" && (
        <div className="memory-decision-block">
          {Boolean(memory.role_metadata.reason) && <p className="memory-decision-reason">理由: {String(memory.role_metadata.reason)}</p>}
          {Array.isArray(memory.role_metadata.rejected_alternatives) && memory.role_metadata.rejected_alternatives.length > 0 && (
            <details>
              <summary>却下した選択肢 ({memory.role_metadata.rejected_alternatives.length})</summary>
              <ul>
                {memory.role_metadata.rejected_alternatives.map((alt: any, idx: number) => (
                  <li key={idx}>{alt.option}{alt.reason ? ` — ${alt.reason}` : ""}</li>
                ))}
              </ul>
            </details>
          )}
        </div>
      )}
      {memory.role_metadata && memory.role === "failure" && (
        <div className="memory-failure-block">
          {Boolean(memory.role_metadata.attempt) && <p className="memory-failure-attempt">試行: {String(memory.role_metadata.attempt)}</p>}
          {Boolean(memory.role_metadata.reason) && <p className="memory-failure-reason">原因: {String(memory.role_metadata.reason)}</p>}
          {Boolean(memory.role_metadata.lesson) && <p className="memory-failure-lesson">教訓: {String(memory.role_metadata.lesson)}</p>}
        </div>
      )}
      <div className="memory-metrics">
        <span>確度 {percent(memory.confidence)}</span>
        <span>顕著性 {percent(memory.salience)}</span>
        {typeof memory.relevance === "number" && <span>関連度 {percent(memory.relevance)}</span>}
        {memory.evidence_count ? <span>Evidence {memory.evidence_count}</span> : null}
      </div>
      {memory.selection_reason && <p className="memory-selection-reason">{memory.selection_reason}</p>}
    </div>
  );
}

function MemoryDetail({ memory, onClose }: { memory: MemoryView; onClose: () => void }) {
  const evidence = useQuery<Array<{ id: string; kind: string; ref?: string; summary?: string; captured_at?: string }>>({
    queryKey: ["/memories", memory.id, "evidence"],
    queryFn: () => api(`/memories/${memory.id}/evidence`),
  });
  const associations = useQuery<any[]>({ queryKey: ["/memories", memory.id, "associations"], queryFn: () => api(`/memories/${memory.id}/associations`) });
  return (
    <section className="card memory-detail" aria-label={t("記憶の詳細")}>
      <div className="memory-detail-header">
        <div>
          <h3>{ROLE_LABEL[memory.role] || memory.role}</h3>
          <p>{SCOPE_LABEL[memory.scope]} · {LIFECYCLE_LABEL[memory.lifecycle] || memory.lifecycle}</p>
        </div>
        <Button variant="ghost" onClick={onClose}>{t("閉じる")}</Button>
      </div>
      <p className="memory-content">{memory.content || memory.gist}</p>
      {memory.role_metadata && (
        <div className="memory-detail-section">
          <h4>Role metadata</h4>
          <pre>{JSON.stringify(memory.role_metadata, null, 2)}</pre>
        </div>
      )}
      <div className="memory-detail-section">
        <h4>Quality</h4>
        <ul>
          <li>confidence {percent(memory.confidence)}</li>
          <li>salience {percent(memory.salience)}</li>
          <li>strength {percent(memory.strength)}</li>
          <li>verification: {memory.verification}</li>
          {memory.task_id && <li>task_id: {memory.task_id}</li>}
          {memory.superseded_by_id && <li>superseded by: {memory.superseded_by_id}</li>}
          {memory.disputed_by_id && <li>disputed by: {memory.disputed_by_id}</li>}
        </ul>
      </div>
      <div className="memory-detail-section">
        <h4>Evidence</h4>
        {evidence.data?.length ? (
          <ul>{evidence.data.map((e) => <li key={e.id}><strong>{e.kind}</strong> · {e.ref} — {e.summary}</li>)}</ul>
        ) : (
          <p>{t("この記憶には Evidence がまだ紐づいていません。")}</p>
        )}
      </div>
      <div className="memory-detail-section">
        <h4>Associations</h4>
        {associations.data?.length ? (
          <ul>{associations.data.map((a) => <li key={a.id}>{a.relation || t("関連")}: {a.connected_memory?.content || a.target_memory_id}</li>)}</ul>
        ) : (
          <p>{t("この記憶には繋がりがまだありません。")}</p>
        )}
      </div>
    </section>
  );
}

export function Memories() {
  const qc = useQueryClient();
  const [q, setQ] = useState("");
  const [scope, setScope] = useState("");
  const [roleFilter, setRoleFilter] = useState("");
  const [probeQuery, setProbeQuery] = useState("");
  const [probe, setProbe] = useState<{ memories: MemoryView[]; debug: any } | null>(null);
  const [selected, setSelected] = useState<MemoryView | null>(null);
  const [showConflicts, setShowConflicts] = useState(false);
  const [rememberOpen, setRememberOpen] = useState(false);

  const groups = useQuery<GroupResponse>({
    queryKey: ["/memories/groups", scope],
    queryFn: () => api(`/memories/groups${scope ? `?scope=${scope}` : ""}`),
  });
  const conflicts = useQuery<ConflictView[]>({
    queryKey: ["/memories/conflicts"],
    queryFn: () => api("/memories/conflicts"),
  });
  const settings = useQuery<any>({ queryKey: ["/settings"], queryFn: () => api("/settings") });

  const refreshAll = () => {
    qc.invalidateQueries({ predicate: (item) => String(item.queryKey[0]).startsWith("/memories") });
  };

  const filteredGroups = useMemo(() => {
    if (!groups.data) return null;
    if (!roleFilter) return groups.data.groups;
    return Object.fromEntries(
      Object.entries(groups.data.groups).filter(([_, items]) =>
        items.some((mem) => mem.role === roleFilter),
      ),
    );
  }, [groups.data, roleFilter]);

  return (
    <main className="page memory-mind-view">
      <Title title="Agent の記憶" sub={t("Memory を分類箱ではなく、役割 (Decision / Failure / Experience / Fact / Preference / Goal) で整理された Agent の記憶として表示します。")} />

      <ErrorBox error={groups.error || conflicts.error || settings.error} />

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
          <h3>{t("形成と想起")}</h3>
          <label className="check">
            <input name="auto" type="checkbox" defaultChecked={settings.data.data.memory_auto_formation !== false} />
            {t("回答後に Task Memory を非同期形成 (Decision / Failure / Experience を Memory に昇格)")}
          </label>
          <p className="memory-auto-note">{t("Memory は役割ベースで保持され、想起時に Decision / Failure を優先します。想起の重み・閾値・検索予算は記憶ネットワークの規模から自動計算されます。")}</p>
          <Button>{t("設定を保存")}</Button>
        </form>
      )}

      <section className="card memory-search-section">
        <div className="memory-toolbar">
          <div className="memory-toolbar-left">
            <Search size={16} />
            <input
              className="search-input"
              aria-label={t("Memory 検索")}
              placeholder={t("例: Postgres migration の方法 / 失敗したテスト / Decision 理由")}
              value={q}
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={async (e) => {
                if (e.key === "Enter" && q.trim()) {
                  const result = await api<{ memories: MemoryView[]; debug: any }>(`/memories?q=${encodeURIComponent(q)}${roleFilter ? `&role=${roleFilter}` : ""}${scope ? `&scope=${scope}` : ""}`);
                  setProbe(result);
                }
              }}
            />
            <select aria-label={t("役割")} value={roleFilter} onChange={(e) => setRoleFilter(e.target.value)}>
              <option value="">{t("すべての役割")}</option>
              {Object.entries(ROLE_LABEL).map(([value, label]) => (
                <option key={value} value={value}>{label}</option>
              ))}
            </select>
            <select aria-label={t("スコープ")} value={scope} onChange={(e) => setScope(e.target.value)}>
              <option value="">{t("すべてのスコープ")}</option>
              {Object.entries(SCOPE_LABEL).map(([value, label]) => (
                <option key={value} value={value}>{label}</option>
              ))}
            </select>
            <Button onClick={refreshAll}><RefreshCw size={14} /> {t("再構築")}</Button>
          </div>
          <div className="memory-toolbar-right">
            <Button variant="ghost" onClick={() => setShowConflicts((v) => !v)}>
              <AlertTriangle size={14} /> {t("Conflicts")} ({conflicts.data?.length ?? 0})
            </Button>
            <Button onClick={() => setRememberOpen(true)}>
              <ListChecks size={14} /> {t("memory.remember")}
            </Button>
          </div>
        </div>
      </section>

      {probe && (
        <section className="card memory-trace-result">
          <header><h3>{t("想起トレース")}</h3></header>
          {probe.debug && (
            <div className="memory-metrics">
              <span>{t("候補")} {probe.debug.considered}</span>
              <span>{t("採用")} {probe.debug.kept}</span>
              <span>{t("経過")} {probe.debug.elapsed_ms}ms</span>
              <span>{t("役割")} {probe.debug.roles?.join(", ") || "-"}</span>
              <span>{t("スコープ")} {probe.debug.scopes?.join(", ") || "-"}</span>
            </div>
          )}
          <div className="memory-grid">
            {(probe.memories || []).map((mem) => <MemoryCard key={mem.id} memory={mem} onSelect={setSelected} />)}
          </div>
        </section>
      )}

      {selected && <MemoryDetail memory={selected} onClose={() => setSelected(null)} />}

      <section className="memory-mind-grid">
        {(["decisions", "failures", "preferences", "goals", "facts", "experiences", "questions", "other"] as const).map((bucket) => {
          const items = filteredGroups?.[bucket] || [];
          if (!items.length) return null;
          return (
            <div className="card memory-mind-section" key={bucket}>
              <header>
                <h3>
                  {bucket === "decisions" && <CheckCircle2 size={16} />}
                  {bucket === "failures" && <AlertTriangle size={16} />}
                  {bucket === "preferences" && <Circle size={16} />}
                  {bucket === "goals" && <Workflow size={16} />}
                  {bucket === "facts" && <Brain size={16} />}
                  {bucket === "experiences" && <History size={16} />}
                  {bucket === "questions" && <Eye size={16} />}
                  {bucket === "other" && <Network size={16} />}
                  {bucket === "decisions" ? t("Active Decisions")
                    : bucket === "failures" ? t("Past failures to avoid")
                    : bucket === "preferences" ? t("User preferences")
                    : bucket === "goals" ? t("Goals / Constraints")
                    : bucket === "facts" ? t("Relevant facts")
                    : bucket === "experiences" ? t("Experiences")
                    : bucket === "questions" ? t("Open questions")
                    : t("Other memory")}
                  <small>{items.length}</small>
                </h3>
              </header>
              <div className="memory-grid">
                {items.map((mem) => <MemoryCard key={mem.id} memory={mem} onSelect={setSelected} />)}
              </div>
            </div>
          );
        })}
        {!groups.isPending && !filteredGroups?.decisions?.length && !filteredGroups?.failures?.length && !filteredGroups?.facts?.length && (
          <div className="card memory-empty">
            <p>{t("まだ記憶がありません。Agent が動作すると、ここに Decision / Failure / Experience が蓄積されます。")}</p>
          </div>
        )}
      </section>

      {showConflicts && (
        <section className="card memory-conflicts">
          <header><h3><AlertTriangle size={16} /> {t("未解決の Conflict")}</h3></header>
          {(conflicts.data || []).length === 0 ? (
            <p>{t("現在 Conflict はありません。")}</p>
          ) : (
            (conflicts.data || []).map((conflict) => (
              <div key={conflict.id} className="memory-conflict-row">
                <div className="memory-conflict-items">
                  {conflict.a && <MemoryCard memory={conflict.a} onSelect={setSelected} />}
                  <span className="memory-conflict-vs">↔</span>
                  {conflict.b && <MemoryCard memory={conflict.b} onSelect={setSelected} />}
                </div>
                <div className="memory-conflict-actions">
                  <p>{conflict.reason || t("両者の主張が対立しています。")}</p>
                  <div className="memory-conflict-buttons">
                    <Button onClick={async () => { await api(`/memories/conflicts/${conflict.id}/resolve`, "POST", { resolution: "prefer_a", prefer: conflict.a?.id }); refreshAll(); }}>
                      <CheckCircle2 size={14} /> {t("A を採用")}
                    </Button>
                    <Button onClick={async () => { await api(`/memories/conflicts/${conflict.id}/resolve`, "POST", { resolution: "prefer_b", prefer: conflict.b?.id }); refreshAll(); }}>
                      <CheckCircle2 size={14} /> {t("B を採用")}
                    </Button>
                    <Button variant="ghost" onClick={async () => { await api(`/memories/conflicts/${conflict.id}/resolve`, "POST", { resolution: "dismiss" }); refreshAll(); }}>
                      <Trash2 size={14} /> {t("保留")}
                    </Button>
                  </div>
                </div>
              </div>
            ))
          )}
        </section>
      )}

      {rememberOpen && (
        <RememberForm
          onClose={() => setRememberOpen(false)}
          onSaved={() => {
            setRememberOpen(false);
            refreshAll();
          }}
        />
      )}

      <section className="card memory-debug">
        <header><h3>{t("想起プローブ")}</h3></header>
        <form onSubmit={async (e) => {
          e.preventDefault();
          try {
            const result = await api<{ memories: MemoryView[]; debug: any }>(`/memories-debug/search?q=${encodeURIComponent(probeQuery.trim())}${roleFilter ? `&role=${roleFilter}` : ""}${scope ? `&scope=${scope}` : ""}`);
            setProbe(result);
          } catch (reason) {
            console.error(reason);
          }
        }}>
          <Field label={t("クエリを入力して想起パイプラインの過程を確認")}>
            <input required value={probeQuery} onChange={(e) => setProbeQuery(e.target.value)} placeholder={t("例: Postgres migration で失敗した方法")} />
          </Field>
          <div className="form-actions"><Button>{t("想起を実行")}</Button></div>
        </form>
      </section>
    </main>
  );
}

function RememberForm({ onClose, onSaved }: { onClose: () => void; onSaved: () => void }) {
  const [content, setContent] = useState("");
  const [role, setRole] = useState("fact");
  const [scope, setScope] = useState("user");
  const [error, setError] = useState<unknown>(null);
  const [saving, setSaving] = useState(false);
  return (
    <div className="card memory-remember-form">
      <header><h3><ListChecks size={16} /> {t("memory.remember")}</h3></header>
      <p className="memory-auto-note">{t("Memory Runtime の Evaluator が既存 Memory との重複 / 矛盾を判定します。Decision / Failure は role_metadata を追加すると、後で想起しやすくなります。")}</p>
      {error ? <ErrorBox error={error} /> : null}
      <Field label={t("内容")}>
        <textarea required value={content} onChange={(e) => setContent(e.target.value)} rows={4} />
      </Field>
      <div className="memory-setting-grid">
        <Field label={t("役割")}>
          <select value={role} onChange={(e) => setRole(e.target.value)}>
            {Object.entries(ROLE_LABEL).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
          </select>
        </Field>
        <Field label={t("スコープ")}>
          <select value={scope} onChange={(e) => setScope(e.target.value)}>
            {Object.entries(SCOPE_LABEL).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
          </select>
        </Field>
      </div>
      <div className="form-actions">
        <Button onClick={async () => {
          setSaving(true);
          try {
            await api("/memory-tools/remember", "POST", { content, role, scope, explicit_user: true });
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
