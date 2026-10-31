import { t } from "@/app/i18n";
import { api } from "@/app/api";
import { queryKeys, useSettings } from "@/lib/queries";
import { Button } from "@/components/button";
import { CardSkeleton } from "@/components/loading";
import { ConfirmButton } from "@/components/modal";
import { ErrorBox, Field, Title } from "@/components/shared";
import { useToast } from "@/components/toast";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Brain,
  GitMerge,
  History,
  Link2,
  ListChecks,
  RefreshCw,
  Search,
  Trash2,
} from "lucide-react";
import { useState } from "react";
import { AssociationsView } from "./AssociationsView";
import { TraceView, type TraceProbe } from "./TraceView";

type MemoryRow = {
  id: string;
  data: {
    content?: string;
    gist?: string;
    pinned?: boolean;
    source?: string;
    // §1's columns, reported straight off the row by `GET /memories`.
    role?: string;
    scope?: string;
    source_kind?: string;
    lifecycle_state?: string;
    superseded_by_id?: string | null;
    disputed_by_id?: string | null;
    confidence?: number;
    // §4: number of edges touching this row, absent when there are none.
    link_count?: number;
  };
  created_at: string;
};

type RevisionRow = {
  id: string;
  created_at: string;
  data?: { memory_id?: string; previous?: Record<string, unknown> };
};

/** §2 lifecycles a row that left retrieval still shows in the list (§3). */
const RETIRED_STATES = ["superseded", "disputed", "archived", "expired"];

function contentOf(row: MemoryRow): string {
  return row.data?.content || row.data?.gist || "";
}

function short(id?: string | null): string {
  return id ? id.slice(0, 8) : "";
}

export function Memories() {
  const qc = useQueryClient();
  const { addToast } = useToast();
  const [q, setQ] = useState("");
  const [probeQuery, setProbeQuery] = useState("");
  const [probe, setProbe] = useState<TraceProbe | null>(null);
  const [editing, setEditing] = useState<MemoryRow | null>(null);
  const [rememberOpen, setRememberOpen] = useState(false);
  const [panel, setPanel] = useState<{
    id: string;
    kind: "associations" | "revisions" | "resolve";
  } | null>(null);

  const list = useQuery<MemoryRow[]>({
    queryKey: ["/memories", q],
    queryFn: () => api(`/memories?q=${encodeURIComponent(q)}`),
  });
  const settings = useSettings();

  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ["/memories"] });
  };

  const togglePanel = (
    id: string,
    kind: "associations" | "revisions" | "resolve",
  ) => {
    setPanel((current) =>
      current && current.id === id && current.kind === kind
        ? null
        : { id, kind },
    );
  };

  const runProbe = async (raw: string) => {
    const query = raw.trim();
    if (!query) return;
    try {
      const result = await api<TraceProbe>(
        `/memories-debug/search?q=${encodeURIComponent(query)}`,
      );
      setProbe(result);
    } catch {
      setProbe({ memories: [] });
    }
  };

  const removeOne = async (id: string) => {
    await api(`/memories/${id}`, "DELETE");
    await refresh();
  };

  return (
    <section className="page memory-mind-view">
      <Title
        title="記憶"
        sub={t(
          "ChatGPTのように、ユーザーについての短い事実を1件ずつ保存します。",
        )}
      />

      <ErrorBox
        error={list.error || settings.error}
        onRetry={() => {
          void list.refetch();
          void settings.refetch();
        }}
      />

      {settings.data && (
        <form
          className="card memory-settings"
          onSubmit={async (e) => {
            e.preventDefault();
            const form = new FormData(e.currentTarget);
            try {
              // The `has_*` flags are server-computed; drop them so the PUT cannot
              // echo secret-presence metadata back as settings.
              const {
                has_secret_id: _has_secret_id,
                has_brave_secret_id: _has_brave_secret_id,
                has_tavily_secret_id: _has_tavily_secret_id,
                has_exa_secret_id: _has_exa_secret_id,
                has_serper_secret_id: _has_serper_secret_id,
                ...rest
              } = settings.data.data;
              await api("/settings", "PUT", {
                ...rest,
                memory_auto_formation: form.get("auto") === "on",
                memory_link_embedding_neighbors: form.get("neighbors") === "on",
                brave_api_key: null,
                tavily_api_key: null,
                exa_api_key: null,
                serper_api_key: null,
              });
              await qc.invalidateQueries({ queryKey: queryKeys.settings });
              addToast({ type: "success", title: t("設定を保存しました") });
            } catch (reason) {
              addToast({
                type: "error",
                title: t("設定を保存できませんでした"),
                message:
                  reason instanceof Error ? reason.message : String(reason),
              });
            }
          }}
        >
          <h3>{t("自動記憶")}</h3>
          <label className="check">
            <input
              name="auto"
              type="checkbox"
              defaultChecked={
                settings.data.data.memory_auto_formation !== false
              }
            />
            {t("回答後にユーザー facts を自動抽出する")}
          </label>
          {/* §4 source (3), the one generation source that is off by default:
              the checkbox writes the existing setting, it does not decide the
              default — the server keeps `false` when the key is absent. */}
          <label className="check">
            <input
              name="neighbors"
              type="checkbox"
              defaultChecked={
                settings.data.data.memory_link_embedding_neighbors === true
              }
            />
            {t("埋め込み近傍からリンクを張る（既定オフ）")}
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
            <Button onClick={refresh}>
              <RefreshCw size={14} /> {t("更新")}
            </Button>
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
          <h3>
            <Brain size={16} /> {t("保存された記憶")}{" "}
            <small>{list.data?.length ?? 0}</small>
          </h3>
        </header>
        {list.isLoading ? (
          <div className="memory-grid" aria-busy="true">
            <CardSkeleton />
            <CardSkeleton />
          </div>
        ) : list.isError ? null : !list.data?.length ? (
          settings.error ? null : (
            <p>
              {t(
                "まだ記憶がありません。「覚える」から追加するか、「○○を覚えて」と話しかけてください。",
              )}
            </p>
          )
        ) : (
          <ul className="memory-grid">
            {list.data.map((mem) => {
              const state = mem.data.lifecycle_state || "established";
              const retired = RETIRED_STATES.includes(state);
              const open = panel && panel.id === mem.id ? panel.kind : null;
              return (
                <li
                  key={mem.id}
                  className="card memory-card"
                  data-role={mem.data.role || "fact"}
                >
                  <div className="memory-card-header">
                    <span
                      className={`memory-role-badge memory-role-${mem.data.role || "fact"}`}
                    >
                      {mem.data.role || "fact"}
                    </span>
                    <span className="memory-scope">
                      {mem.data.scope || "user"}
                    </span>
                    {/* §1's `lifecycle_state`, always shown once: the retired
                        states get the emphatic badge, the retrievable ones the
                        quiet chip. No state is ever hidden. */}
                    {retired ? (
                      <span className={`memory-${state}`}>{state}</span>
                    ) : (
                      <span className="memory-lifecycle">{state}</span>
                    )}
                    <span className="memory-lifecycle">
                      {mem.data.source_kind || "agent"}
                    </span>
                    <span className="memory-lifecycle">
                      {t("信頼度")}{" "}
                      {Number(mem.data.confidence ?? 0.7).toFixed(2)}
                    </span>
                    {mem.data.pinned ? (
                      <span className="memory-lifecycle">{t("固定")}</span>
                    ) : null}
                    {mem.data.link_count ? (
                      <span className="memory-lifecycle">
                        {t("関連")} {mem.data.link_count}
                      </span>
                    ) : null}
                  </div>
                  <p className="memory-content">{contentOf(mem)}</p>
                  <div className="memory-metrics">
                    {mem.data.source ? <span>{mem.data.source}</span> : null}
                    {mem.data.superseded_by_id ? (
                      <span className="memory-superseded">
                        → {short(mem.data.superseded_by_id)}
                      </span>
                    ) : null}
                    {mem.data.disputed_by_id ? (
                      <span className="memory-disputed">
                        ↔ {short(mem.data.disputed_by_id)}
                      </span>
                    ) : null}
                  </div>
                  <div className="form-actions">
                    <Button
                      variant="ghost"
                      onClick={() => togglePanel(mem.id, "associations")}
                    >
                      <Link2 size={14} /> {t("関連")}
                    </Button>
                    <Button
                      variant="ghost"
                      onClick={() => togglePanel(mem.id, "revisions")}
                    >
                      <History size={14} /> {t("履歴")}
                    </Button>
                    {state === "disputed" && (
                      <Button
                        variant="ghost"
                        onClick={() => togglePanel(mem.id, "resolve")}
                      >
                        <GitMerge size={14} /> {t("矛盾を解決")}
                      </Button>
                    )}
                    <Button variant="ghost" onClick={() => setEditing(mem)}>
                      {t("編集")}
                    </Button>
                    <ConfirmButton
                      variant="ghost"
                      title={t("記憶を忘れる")}
                      message={t("この記憶を削除しますか？ 元に戻せません。")}
                      confirmLabel={t("忘れる")}
                      onConfirm={async () => {
                        try {
                          await removeOne(mem.id);
                          addToast({
                            type: "success",
                            title: t("記憶を削除しました"),
                          });
                        } catch (reason) {
                          addToast({
                            type: "error",
                            title: t("記憶を削除できませんでした"),
                            message:
                              reason instanceof Error
                                ? reason.message
                                : String(reason),
                          });
                        }
                      }}
                    >
                      <Trash2 size={14} /> {t("忘れる")}
                    </ConfirmButton>
                  </div>
                  {open === "associations" && (
                    <AssociationsView
                      memoryId={mem.id}
                      onClose={() => setPanel(null)}
                    />
                  )}
                  {open === "revisions" && (
                    <RevisionsPanel
                      row={mem}
                      onClose={() => setPanel(null)}
                      onRestored={() => {
                        setPanel(null);
                        refresh();
                      }}
                    />
                  )}
                  {open === "resolve" && (
                    <ResolvePanel
                      row={mem}
                      rows={list.data || []}
                      onClose={() => setPanel(null)}
                      onResolved={() => {
                        setPanel(null);
                        refresh();
                      }}
                    />
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </section>

      {editing && (
        <EditForm
          row={editing}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            refresh();
          }}
        />
      )}

      {rememberOpen && (
        <RememberForm
          onClose={() => setRememberOpen(false)}
          onSaved={() => {
            setRememberOpen(false);
            refresh();
          }}
        />
      )}

      <section className="card memory-debug">
        <header>
          <h3>{t("想起プローブ")}</h3>
        </header>
        <form
          onSubmit={async (e) => {
            e.preventDefault();
            await runProbe(probeQuery);
          }}
        >
          <Field label={t("クエリを入力して一致を確認")}>
            <input
              required
              value={probeQuery}
              onChange={(e) => setProbeQuery(e.target.value)}
              placeholder={t("例: 日本語")}
            />
          </Field>
          <div className="form-actions">
            <Button>{t("想起を実行")}</Button>
          </div>
        </form>
        {probe && (
          <TraceView
            probe={probe}
            query={probeQuery}
            onClose={() => setProbe(null)}
          />
        )}
      </section>
    </section>
  );
}

/**
 * §1's revision history over `GET /memories/{key}/revisions`, with §1's own
 * way back: `POST /memories/{key}/restore/{revision}`. Restoring never drops
 * the history it came from — the restore writes another revision on top.
 */
function RevisionsPanel({
  row,
  onClose,
  onRestored,
}: {
  row: MemoryRow;
  onClose: () => void;
  onRestored: () => void;
}) {
  const revisions = useQuery<RevisionRow[]>({
    queryKey: ["/memories", row.id, "revisions"],
    queryFn: () => api(`/memories/${row.id}/revisions`),
  });
  const [error, setError] = useState<unknown>(null);
  const restore = async (revisionId: string) => {
    try {
      await api(`/memories/${row.id}/restore/${revisionId}`, "POST");
      onRestored();
    } catch (reason) {
      setError(reason);
    }
  };
  return (
    <div className="memory-detail">
      <div className="memory-detail-header">
        <h4>
          <History size={13} /> {t("履歴")}
        </h4>
        <Button variant="ghost" onClick={onClose}>
          {t("閉じる")}
        </Button>
      </div>
      <ErrorBox error={revisions.error || error} />
      {!revisions.data?.length ? (
        <p className="memory-empty">
          {t("このMemoryの履歴はまだありません。")}
        </p>
      ) : (
        <ul className="memory-score-list">
          {revisions.data.map((rev) => {
            const previous = (rev.data?.previous || {}) as {
              content?: string;
              lifecycle_state?: string;
            };
            return (
              <li key={rev.id}>
                <span>
                  {rev.created_at}
                  {previous.lifecycle_state
                    ? ` · ${previous.lifecycle_state}`
                    : ""}
                  {" — "}
                  {String(previous.content || "").slice(0, 60)}
                </span>
                <Button variant="ghost" onClick={() => restore(rev.id)}>
                  {t("復元")}
                </Button>
              </li>
            );
          })}
        </ul>
      )}
      <p className="memory-auto-note">
        {t("復元しても過去の改訂は残ります。")}
      </p>
    </div>
  );
}

/**
 * §3 `resolve_conflict` from the row that the list already shows as
 * `disputed`.
 *
 * The first button is the one that matters: it sends **no `prefer`**, so the
 * winner is decided by §3's own default (the memory `disputed_by_id` points
 * at — the one already stored). Pre-selecting a side in the form would have
 * hidden that the default is a spec rule and not an accident of this UI.
 */
function ResolvePanel({
  row,
  rows,
  onClose,
  onResolved,
}: {
  row: MemoryRow;
  rows: MemoryRow[];
  onClose: () => void;
  onResolved: () => void;
}) {
  const otherId = row.data.disputed_by_id || "";
  const other = rows.find((item) => item.id === otherId);
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);

  const settle = async (body: { resolution: string; prefer?: string }) => {
    setBusy(true);
    try {
      await api(`/memories/${row.id}/resolve`, "POST", body);
      onResolved();
    } catch (reason) {
      setError(reason);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="memory-conflicts">
      <div className="memory-conflict-row">
        <div className="memory-conflict-items">
          <span>
            {contentOf(row).slice(0, 60) || short(row.id)}
            <small> {row.data.lifecycle_state}</small>
          </span>
          <span className="memory-conflict-vs">vs</span>
          <span>
            {other ? contentOf(other).slice(0, 60) : short(otherId)}
            <small> {other?.data.lifecycle_state || ""}</small>
          </span>
        </div>
        <div className="memory-conflict-actions">
          <p className="memory-auto-note">
            {t(
              "既定は保存済み側（disputed_by_id の相手）が勝ち、却下した側は superseded になります。両側に改訂が残ります。",
            )}
          </p>
          <div className="memory-conflict-buttons">
            <Button
              onClick={() => settle({ resolution: "resolved" })}
              disabled={busy}
            >
              {t("既定どおり採用")}
            </Button>
            <Button
              variant="ghost"
              onClick={() => settle({ resolution: "resolved", prefer: row.id })}
              disabled={busy}
            >
              {t("この Memory を採用")}
            </Button>
            {otherId && (
              <Button
                variant="ghost"
                onClick={() =>
                  settle({ resolution: "resolved", prefer: otherId })
                }
                disabled={busy}
              >
                {t("相手を採用")}
              </Button>
            )}
            <Button
              variant="ghost"
              onClick={() => settle({ resolution: "dismiss" })}
              disabled={busy}
            >
              {t("失効させる")}
            </Button>
          </div>
          <ErrorBox error={error} />
        </div>
      </div>
      <div className="form-actions">
        <Button variant="ghost" onClick={onClose}>
          {t("閉じる")}
        </Button>
      </div>
    </div>
  );
}

function RememberForm({
  onClose,
  onSaved,
}: {
  onClose: () => void;
  onSaved: () => void;
}) {
  const [content, setContent] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [saving, setSaving] = useState(false);
  return (
    <div className="card memory-remember-form">
      <header>
        <h3>
          <ListChecks size={16} /> {t("記憶を追加")}
        </h3>
      </header>
      <p className="memory-auto-note">
        {t(
          "短い事実を1件ずつ保存します（200字以内推奨）。重複は自動で無視されます。",
        )}
      </p>
      {error ? <ErrorBox error={error} /> : null}
      <Field label={t("内容")}>
        <textarea
          required
          value={content}
          onChange={(e) => setContent(e.target.value)}
          rows={3}
          maxLength={1000}
        />
      </Field>
      <div className="form-actions">
        <Button
          onClick={async () => {
            setSaving(true);
            try {
              await api("/memories", "POST", { content });
              onSaved();
            } catch (reason) {
              setError(reason);
            } finally {
              setSaving(false);
            }
          }}
          disabled={saving || !content.trim()}
        >
          {t("保存")}
        </Button>
        <Button variant="ghost" onClick={onClose}>
          {t("キャンセル")}
        </Button>
      </div>
    </div>
  );
}

function EditForm({
  row,
  onClose,
  onSaved,
}: {
  row: MemoryRow;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [content, setContent] = useState(contentOf(row));
  const [error, setError] = useState<unknown>(null);
  const [saving, setSaving] = useState(false);
  return (
    <div className="card memory-remember-form">
      <header>
        <h3>{t("記憶を編集")}</h3>
      </header>
      {error ? <ErrorBox error={error} /> : null}
      <Field label={t("内容")}>
        <textarea
          required
          value={content}
          onChange={(e) => setContent(e.target.value)}
          rows={3}
          maxLength={1000}
        />
      </Field>
      <div className="form-actions">
        <Button
          onClick={async () => {
            setSaving(true);
            try {
              await api(`/memories/${row.id}`, "PATCH", { content });
              onSaved();
            } catch (reason) {
              setError(reason);
            } finally {
              setSaving(false);
            }
          }}
          disabled={saving || !content.trim()}
        >
          {t("保存")}
        </Button>
        <Button variant="ghost" onClick={onClose}>
          {t("キャンセル")}
        </Button>
      </div>
    </div>
  );
}
