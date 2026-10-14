import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Brain, CheckCircle2, History, Link2, Search, X } from "lucide-react";
import { api } from "@/lib/api";
import { t, tf } from "@/lib/i18n";
import { useSearch } from "@/lib/useSearch";
import { Card, Field, Row, Section } from "@/components/layout";
import { EmptyState, ErrorBadge, ErrorBox, ListSkeleton } from "@/components/shared";
import { DeleteIconButton, SearchEmptyState, SearchForm, SectionHeading } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import { Input, Select, Textarea } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import type { MemoryView } from "@/lib/types";
import { AssociationsView } from "./AssociationsView";

/**
 * Spec §2 `lifecycles` → badge tone, so every state §3 can put a row in
 * (`superseded` / `disputed` / `archived` / `expired` included) renders with
 * a meaning of its own instead of falling through one `active`-or-else
 * branch. The label itself stays the row's real `lifecycle_state`.
 */
const LIFECYCLE_TONE: Record<string, "success" | "warning" | "danger" | "neutral" | "info"> = {
  active: "success",
  established: "success",
  candidate: "info",
  disputed: "danger",
  superseded: "neutral",
  expired: "neutral",
  archived: "neutral",
};

/** §2 `roles`, in §2's order — the values the form is allowed to write. */
const ROLES = [
  "fact", "decision", "preference", "goal", "constraint", "experience",
  "failure", "solution", "observation", "hypothesis", "question", "unknown",
] as const;

/** §2 `scopes`, in §2's order. */
const SCOPES = ["working", "task", "project", "user", "world"] as const;

export function MemoryPane() {
  const queryClient = useQueryClient();
  const memories = useQuery({ queryKey: ["memories"], queryFn: api.listMemories });
  const [draftTitle, setDraftTitle] = useState("");
  const [draftBody, setDraftBody] = useState("");
  const [draftRole, setDraftRole] = useState("fact");
  const [draftScope, setDraftScope] = useState("user");
  const search = useSearch((q, n) => api.searchMemories(q, n), 20);
  /** The §4 screen currently open, and the row a pointer badge jumped to. */
  const [associationsFor, setAssociationsFor] = useState<MemoryView | null>(null);
  const [focusId, setFocusId] = useState<string | null>(null);

  const rows = useMemo(() => memories.data ?? [], [memories.data]);
  const byId = useMemo(() => new Map(rows.map((row) => [row.id, row])), [rows]);
  const titles = useMemo(
    () => Object.fromEntries(rows.map((row) => [row.id, row.title])),
    [rows],
  );

  // §1's `superseded_by_id` / `disputed_by_id` are only useful if the row they
  // name can be reached. The list is the 200 most recently updated rows, so a
  // target may simply not be on the page — the badge then renders as text
  // rather than as a link that leads nowhere.
  useEffect(() => {
    if (!focusId) return;
    document
      .getElementById(`memory-${focusId}`)
      ?.scrollIntoView({ block: "center", behavior: "smooth" });
    const timer = window.setTimeout(() => setFocusId(null), 1600);
    return () => window.clearTimeout(timer);
  }, [focusId]);

  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ["memories"] });
    queryClient.invalidateQueries({ queryKey: ["memory_associations"] });
  };

  const create = useMutation({
    mutationFn: () => api.createMemory({
      title: draftTitle, body: draftBody, role: draftRole, scope: draftScope,
    }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["memories"] });
      setDraftTitle(""); setDraftBody("");
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteMemory(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["memories"] }),
  });

  const verify = useMutation({
    mutationFn: (id: string) => api.verifyMemory(id, "verified"),
    onSuccess: refresh,
    onError: refresh,
  });

  // A rejected search used to throw out of this handler into an unhandled
  // rejection: the list simply stayed empty with nothing to say why, while the
  // ingest mutations beside it did surface their errors.
  // State lives in `useSearch` now; this keeps the shared submit handler name.
  const runSearch = () => void search.run();

  const openAssociations = (id: string) => {
    const target = byId.get(id);
    if (target) setAssociationsFor(target);
    setFocusId(id);
  };

  // The card holds the row as it looked when it was opened; re-read it from
  // the list so a title or lifecycle that just changed (a resolve, a restore)
  // is not shown stale while the edges underneath it refresh.
  const associationsSubject =
    associationsFor ? (byId.get(associationsFor.id) ?? associationsFor) : null;

  return (
    <Section>
      <Card title={t("Memory を追加")} icon={<Brain size={13} />}>
        <div className="grid gap-2 sm:grid-cols-[2fr_1fr_1fr]">
          <Input
            placeholder={t("タイトル")}
            value={draftTitle}
            onChange={(e) => setDraftTitle(e.target.value)}
          />
          <Select value={draftRole} onChange={(e) => setDraftRole(e.target.value)}>
            {ROLES.map((role) => (
              <option key={role} value={role}>
                {role}
              </option>
            ))}
          </Select>
          <Select value={draftScope} onChange={(e) => setDraftScope(e.target.value)}>
            {SCOPES.map((scope) => (
              <option key={scope} value={scope}>
                {scope}
              </option>
            ))}
          </Select>
        </div>
        <Textarea placeholder={t("本文…")} value={draftBody} onChange={(e) => setDraftBody(e.target.value)} />
        <Row>
          <Button
            variant="primary"
            disabled={!draftTitle || create.isPending}
            onClick={() => create.mutate()}
          >
            <Brain size={12} /> {t("作成")}
          </Button>
          {create.isError && <ErrorBadge error={create.error} />}
        </Row>
      </Card>

      <Card title={t("検索")} icon={<Search size={13} />}>
        <Field hint={t("agent は毎回の Run 開始時に Memory を検索し、関連するものをコンテキストへ注入します。")}>
          <SearchForm
            value={search.query}
            onChange={search.setQuery}
            onSubmit={runSearch}
            searching={search.searching}
            placeholder={t("キーワード (例: rust, deploy, postgres)")}
          />
        </Field>
        {search.searchError != null && <ErrorBox error={search.searchError} />}
        {search.hits && search.hits.length === 0 && (
          <SearchEmptyState
            compact
            title={t("該当なし")}
            onClear={() => {
              search.setQuery("");
              search.reset();
            }}
          />
        )}
        {search.hits && search.hits.length > 0 && (
          <ul className="flex flex-col gap-2 text-xs">
            {search.hits.map((h) => (
              <li key={h.id} className="border-t border-line pt-2 first:border-0 first:pt-0">
                <Row className="gap-1.5">
                  <strong className="text-sm font-medium text-fg">{h.title}</strong>
                  <Badge size="sm" className="font-mono">{h.role}</Badge>
                  <Badge size="sm" className="font-mono">{h.scope}</Badge>
                  <Badge size="sm" className="font-mono">
                    {tf("score {n}", { n: h.score.toFixed(3) })}
                  </Badge>
                  {h.selection_reason && (
                    // §4's own value: the row came in on an edge, not on the
                    // query. Shown untranslated so it matches `debug.memory_ids`.
                    <Badge size="sm" tone="brand" title={t("クエリに一致せず、リンクから展開された行（§4）")}>
                      {h.selection_reason}
                    </Badge>
                  )}
                </Row>
                <p className="m-0 text-muted">{h.content.slice(0, 160)}</p>
              </li>
            ))}
          </ul>
        )}
      </Card>

      {associationsSubject && (
        <AssociationsView
          subject={associationsSubject}
          titles={titles}
          onClose={() => setAssociationsFor(null)}
          onOpen={openAssociations}
        />
      )}

      <SectionHeading>{t("登録済み Memory")}</SectionHeading>
      {memories.isPending ? (
        <ListSkeleton />
      ) : memories.isError ? (
        <ErrorBox error={memories.error} />
      ) : rows.length === 0 ? (
        <EmptyState icon={<Brain size={22} />} title={t("まだ Memory がありません。")} />
      ) : (
        rows.map((m: MemoryView) => (
          <Card
            key={m.id}
            id={`memory-${m.id}`}
            className={focusId === m.id ? "border-brand" : undefined}
          >
            <Row align="between">
              <strong className="text-sm font-medium text-fg">{m.title}</strong>
              <Row className="flex-nowrap gap-1">
                <Badge size="sm" className="font-mono">{m.role}</Badge>
                <Badge size="sm" className="font-mono">{m.scope}</Badge>
                <Badge size="sm" className="font-mono">{m.source_kind}</Badge>
                <Badge
                  tone={LIFECYCLE_TONE[m.lifecycle_state] ?? "neutral"}
                  size="sm"
                >
                  {m.lifecycle_state}
                </Badge>
                {m.pinned && <Badge tone="brand" size="sm">{t("ピン留め")}</Badge>}
                {m.verification === "verified" && (
                  <Badge tone="success" size="sm">{t("検証済み")}</Badge>
                )}
                {m.verification === "pending" && (
                  <Badge tone="info" size="sm">{t("検証待ち")}</Badge>
                )}
                {m.verification === "disputed" && (
                  <Badge tone="danger" size="sm">{t("検証で矛盾")}</Badge>
                )}
                {m.verification === "rejected" && (
                  <Badge tone="warning" size="sm">{t("検証却下")}</Badge>
                )}
                {m.link_count ? (
                  <Badge tone="neutral" size="sm" className="font-mono">
                    {tf("{n} リンク", { n: m.link_count })}
                  </Badge>
                ) : null}
              </Row>
            </Row>
            <p className="m-0 text-sm text-muted">{m.body}</p>
            <Row className="text-2xs text-faint">
              <span className="font-mono">{tf("confidence {n}", { n: m.confidence.toFixed(2) })}</span>
              <span className="font-mono">{tf("salience {n}", { n: m.salience.toFixed(2) })}</span>
              <span className="font-mono">{tf("strength {n}", { n: m.strength.toFixed(2) })}</span>
              <span>{new Date(m.updated_at).toLocaleString()}</span>
            </Row>
            {(m.superseded_by_id || m.disputed_by_id) && (
              <Row className="gap-1.5">
                {m.superseded_by_id && (
                  <Pointer
                    label={t("置換した Memory")}
                    targetId={m.superseded_by_id}
                    targetTitle={titles[m.superseded_by_id]}
                    tone="neutral"
                    onGo={openAssociations}
                  />
                )}
                {m.disputed_by_id && (
                  <Pointer
                    label={t("矛盾する Memory")}
                    targetId={m.disputed_by_id}
                    targetTitle={titles[m.disputed_by_id]}
                    tone="danger"
                    onGo={openAssociations}
                  />
                )}
              </Row>
            )}
            {m.disputed_by_id && (
              <ResolveConflict
                memory={m}
                otherTitle={titles[m.disputed_by_id]}
                onResolved={refresh}
              />
            )}
            {verify.isError && <ErrorBox error={verify.error} className="text-xs" />}
            <Row align="end">
              <Button size="sm" disabled={verify.isPending} onClick={() => verify.mutate(m.id)} title={t("有効としてマーク")}>
                {verify.isPending ? <Spinner size={12} /> : <CheckCircle2 size={12} />} {t("検証する")}
              </Button>
              <Button
                size="icon-sm"
                onClick={() => void search.run(m.title)}
                title={t("編集せずに検索")}
                aria-label={t("編集せずに検索")}
              >
                <Search size={13} />
              </Button>
              <Button
                size="sm"
                onClick={() => setAssociationsFor(m)}
                title={t("リンクを表示")}
                aria-label={t("リンクを表示")}
              >
                <Link2 size={12} />
                {m.link_count ? (
                  <span className="font-mono text-2xs">{m.link_count}</span>
                ) : null}
              </Button>
              <RevisionsButton
                id={m.id}
                onRestored={refresh}
              />
              <DeleteIconButton
                title={t("Memory を削除しますか？")}
                description={m.title}
                pending={remove.isPending}
                onDelete={() => remove.mutate(m.id)}
              />
            </Row>
          </Card>
        ))
      )}
    </Section>
  );
}

/**
 * §1's `superseded_by_id` / `disputed_by_id` rendered as something you can go
 * to rather than as an opaque UUID.
 *
 * The label is the row's title when the list holds it and a shortened id when
 * it does not — the pane loads the 200 most recent rows, so the target is not
 * guaranteed to be among them, and inventing a title for an absent row would
 * be worse than showing the id it really has.
 */
function Pointer({
  label,
  targetId,
  targetTitle,
  tone,
  onGo,
}: {
  label: string;
  targetId: string;
  targetTitle: string | undefined;
  tone: "neutral" | "danger";
  onGo: (id: string) => void;
}) {
  const known = targetTitle !== undefined;
  const shown = targetTitle ?? `${targetId.slice(0, 8)}…`;
  const inner = (
    <>
      <Badge size="sm" tone={tone}>
        {label}
      </Badge>
      <span className={known ? "min-w-0 truncate text-brand" : "min-w-0 truncate font-mono text-muted"}>
        {shown}
      </span>
    </>
  );
  if (!known) {
    return (
      <span className="inline-flex max-w-full items-center gap-1.5" title={targetId}>
        {inner}
      </span>
    );
  }
  return (
    <button
      type="button"
      className="inline-flex max-w-full cursor-pointer items-center gap-1.5 rounded-md border border-line bg-inset px-2 py-1 text-start transition-colors hover:bg-hover"
      title={tf("{name} を開く", { name: shown })}
      onClick={() => onGo(targetId)}
    >
      {inner}
    </button>
  );
}

/**
 * §3 `resolve_conflict`, offered only on the row §3's `dispute` actually
 * marked.
 *
 * The three buttons map onto the transition exactly as §3 states it, and
 * nothing here rewrites it:
 *
 * * 「既定どおり」 passes no `prefer`, so **the already-stored side wins** —
 *   the rule §3 states, not a choice the UI gets to make silently.
 * * 「この Memory を採用」 is the only button that passes a `prefer`, and it
 *   does so because the user named that side. The loser is not deleted either
 *   way; it becomes `superseded`.
 * * 「この Memory を却下」 resolves as a dismissal, which is the one path §3
 *   gives `expired` instead of `superseded`.
 *
 * Both sides keep their revision: `resolve_conflict` writes one per row before
 * any column moves, and the UI never calls it with anything that would skip
 * that.
 */
function ResolveConflict({
  memory,
  otherTitle,
  onResolved,
}: {
  memory: MemoryView;
  otherTitle: string | undefined;
  onResolved: () => void;
}) {
  const { confirm, dialog } = useConfirm();
  const [error, setError] = useState<unknown>(null);
  const [pending, setPending] = useState(false);
  const [outcome, setOutcome] = useState<string | null>(null);

  const resolve = async (
    description: string,
    prefer: string | null,
    resolution: string,
  ) => {
    if (!(await confirm({
      title: t("矛盾を解決しますか？"),
      description,
      confirmLabel: t("解決"),
      tone: "danger",
    }))) return;
    setPending(true);
    setError(null);
    try {
      const done = await api.resolveConflict({
        conflict_id: memory.id,
        resolution,
        prefer,
      });
      // Say what actually happened rather than what was asked for: `winner` /
      // `loser_state` come back from the transition, so the line cannot drift
      // from §3 the way a message built from the button's label could.
      const adoptedThis = done.winner === memory.id;
      setOutcome(
        tf("{winner} を採用し、{loser} は {state} になりました。", {
          winner: adoptedThis ? t("この Memory") : otherTitle ?? done.winner,
          loser: adoptedThis ? otherTitle ?? done.loser : t("この Memory"),
          state: done.loser_state,
        }),
      );
      // The pair moved together: both rows' lifecycles, both §3 columns, and
      // §4's mirror edges all changed in one transaction on the Rust side.
      onResolved();
    } catch (err) {
      setError(err);
    } finally {
      setPending(false);
    }
  };

  const other = otherTitle ?? memory.disputed_by_id ?? "";
  return (
    <div className="flex flex-col gap-1.5 border-t border-line pt-2">
      <Row className="gap-1.5">
        <Badge size="sm" tone="danger">
          {t("未解決の矛盾")}
        </Badge>
        <span className="text-2xs text-faint">{tf("相手: {name}", { name: other })}</span>
      </Row>
      {error != null && <ErrorBox error={error} className="text-xs" />}
      {outcome && <p className="m-0 text-xs text-muted">{outcome}</p>}
      <Row className="gap-1.5">
        <Button
          size="sm"
          disabled={pending}
          onClick={() =>
            void resolve(
              tf("相手の「{name}」を採用し、この Memory は superseded になります。", { name: other }),
              null,
              "resolve",
            )
          }
          title={t("既定: 保存済み側が勝つ（§3）")}
        >
          {t("既定どおり解決")}
        </Button>
        <Button
          size="sm"
          disabled={pending}
          onClick={() =>
            void resolve(
              tf("この Memory を採用し、相手の「{name}」は superseded になります。", { name: other }),
              memory.id,
              "resolve",
            )
          }
        >
          {t("この Memory を採用")}
        </Button>
        <Button
          size="sm"
          variant="ghost-danger"
          disabled={pending}
          onClick={() =>
            void resolve(
              tf("相手の「{name}」を採用し、この Memory は expired になります。", { name: other }),
              null,
              "dismiss",
            )
          }
        >
          {t("この Memory を却下")}
        </Button>
        {pending && <Spinner size={12} />}
      </Row>
      <p className="m-0 text-xs text-muted">
        {t("どちらも行は削除されず、両側に改訂が残ります（§3）。")}
      </p>
      {dialog}
    </div>
  );
}

function RevisionsButton({ id, onRestored }: { id: string; onRestored: () => void }) {
  const { confirm, dialog } = useConfirm();
  const [open, setOpen] = useState(false);
  const revisions = useQuery({
    queryKey: ["memory_revisions", id],
    queryFn: () => api.listMemoryRevisions(id),
    enabled: open,
  });

  const restore = useMutation({
    mutationFn: (revisionId: string) => api.restoreMemoryRevision(id, revisionId),
    onSuccess: () => { setOpen(false); onRestored(); },
  });

  if (!open) {
    return (
      <Button
        size="icon-sm"
        onClick={() => setOpen(true)}
        title={t("改訂履歴")}
        aria-label={t("改訂履歴")}
      >
        <History size={13} />
      </Button>
    );
  }
  return (
    <div className="flex w-full flex-col gap-1.5 border-t border-line pt-2 text-xs">
      {revisions.isPending && (
        <span className="inline-flex items-center gap-1.5 text-xs text-muted">
          <Spinner size={12} /> {t("読み込み中…")}
        </span>
      )}
      {!revisions.isPending && (revisions.data ?? []).length === 0 && (
        <span className="text-faint">{t("まだ改訂がありません。")}</span>
      )}
      {restore.isError && <ErrorBox error={restore.error} className="text-xs" />}
      {revisions.isError && <ErrorBox error={revisions.error} className="text-xs" />}
      {(revisions.data ?? []).map((r) => (
        <Row key={r.id} align="between">
          <span className="text-muted">{new Date(r.created_at).toLocaleString()}</span>
          <Button
            size="sm"
            disabled={restore.isPending}
            onClick={async () => {
              // Overwrites the current body with no undo, so it confirms like any
              // other destructive action rather than firing on one click.
              if (await confirm({
                title: t("この版に戻しますか？"),
                description: tf("{at} の内容で上書きされます。", {
                  at: new Date(r.created_at).toLocaleString(),
                }),
                confirmLabel: t("復元する"),
                tone: "danger",
              })) {
                restore.mutate(r.id);
              }
            }}
          >
            {t("復元する")}
          </Button>
        </Row>
      ))}
      <Row>
        <Button size="sm" onClick={() => setOpen(false)}>
          <X size={12} /> {t("閉じる")}
        </Button>
        {dialog}
      </Row>
    </div>
  );
}
