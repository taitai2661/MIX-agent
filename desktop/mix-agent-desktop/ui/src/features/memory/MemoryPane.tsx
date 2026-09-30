import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Brain, CheckCircle2, History, Search, Trash2, X } from "lucide-react";
import { api } from "@/lib/api";
import { t, tf } from "@/lib/i18n";
import { Card, Field, Row, Section } from "@/components/layout";
import { EmptyState, ErrorBadge } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Select, Textarea } from "@/components/ui/input";
import type { MemoryRevisionView, MemoryView } from "@/lib/types";

export function MemoryPane() {
  const queryClient = useQueryClient();
  const memories = useQuery({ queryKey: ["memories"], queryFn: api.listMemories });
  const [draftTitle, setDraftTitle] = useState("");
  const [draftBody, setDraftBody] = useState("");
  const [draftRole, setDraftRole] = useState("fact");
  const [draftScope, setDraftScope] = useState("user");
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<MemoryView[] | null>(null);

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
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["memories"] }),
    onError: () => queryClient.invalidateQueries({ queryKey: ["memories"] }),
  });

  const search = async () => setHits(await api.searchMemories(query, 20));

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
            <option value="fact">Fact</option>
            <option value="decision">Decision</option>
            <option value="failure">Failure</option>
            <option value="experience">Experience</option>
            <option value="user">User</option>
          </Select>
          <Select value={draftScope} onChange={(e) => setDraftScope(e.target.value)}>
            <option value="user">User</option>
            <option value="project">Project</option>
            <option value="world">World</option>
            <option value="working">Working</option>
            <option value="task">Task</option>
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

      <Card title={t("検索")}>
        <Field hint={t("agent は毎回の Run 開始時に Memory を検索し、関連するものをコンテキストへ注入します。")}>
          <Row className="gap-1.5">
            <Input
              className="min-w-0 flex-1"
              placeholder={t("キーワード (例: rust, deploy, postgres)")}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && query.trim()) void search();
              }}
            />
            <Button disabled={!query.trim()} onClick={() => void search()}>
              <Search size={12} /> {t("検索")}
            </Button>
          </Row>
        </Field>
        {hits && (
          <ul className="m-0 flex list-none flex-col gap-2 p-0 text-xs">
            {hits.length === 0 && <li className="text-faint">{t("該当なし")}</li>}
            {hits.map((h) => (
              <li key={h.id} className="border-t border-line pt-2 first:border-0 first:pt-0">
                <Row className="gap-1.5">
                  <strong className="text-fg">{h.title}</strong>
                  <Badge size="sm" className="font-mono">{h.role}</Badge>
                </Row>
                <p className="m-0 text-muted">{h.body.slice(0, 160)}</p>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <h2 className="text-sm font-semibold text-fg">{t("登録済み Memory")}</h2>
      {(memories.data ?? []).length === 0 ? (
        <EmptyState icon={<Brain size={22} />} title={t("まだ Memory がありません。")} />
      ) : (
        (memories.data ?? []).map((m: MemoryView) => (
          <Card key={m.id}>
            <Row align="between">
              <strong className="text-sm text-fg">{m.title}</strong>
              <Row className="flex-nowrap gap-1">
                <Badge size="sm" className="font-mono">{m.role}</Badge>
                <Badge size="sm" className="font-mono">{m.scope}</Badge>
                <Badge
                  tone={m.lifecycle_state === "active" ? "success" : "warning"}
                  size="sm"
                >
                  {m.lifecycle_state}
                </Badge>
                {m.verification === "verified" && (
                  <Badge tone="success" size="sm">verified</Badge>
                )}
              </Row>
            </Row>
            <p className="m-0 text-sm text-muted">{m.body}</p>
            <Row align="end">
              <Button size="sm" onClick={() => verify.mutate(m.id)} title={t("Mark as still valid")}>
                <CheckCircle2 size={11} /> {t("verify")}
              </Button>
              <Button
                size="sm"
                onClick={() => setQuery(m.title)}
                title={t("Search instead of editing")}
                aria-label={t("Search instead of editing")}
              >
                <Search size={11} />
              </Button>
              <RevisionsButton
                id={m.id}
                onRestored={() => queryClient.invalidateQueries({ queryKey: ["memories"] })}
              />
              <Button
                size="sm"
                className="text-danger hover:text-danger"
                onClick={() => {
                  if (confirm(tf("Delete \"{title}\"?", { title: m.title }))) remove.mutate(m.id);
                }}
              >
                <Trash2 size={11} />
              </Button>
            </Row>
          </Card>
        ))
      )}
    </Section>
  );
}

function RevisionsButton({ id, onRestored }: { id: string; onRestored: () => void }) {
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
      <Button size="sm" onClick={() => setOpen(true)} title={t("Revision history")} aria-label={t("Revision history")}>
        <History size={12} />
      </Button>
    );
  }
  return (
    <div className="flex w-full flex-col gap-1.5 border-t border-line pt-2 text-xs">
      {(revisions.data ?? []).length === 0 && (
        <span className="text-faint">{t("No revisions yet.")}</span>
      )}
      {(revisions.data ?? []).map((r: MemoryRevisionView) => (
        <Row key={r.id} align="between">
          <span className="text-muted">{new Date(r.created_at).toLocaleString()}</span>
          <Button size="sm" onClick={() => restore.mutate(r.id)}>{t("restore")}</Button>
        </Row>
      ))}
      <Row>
        <Button size="sm" onClick={() => setOpen(false)}>
          <X size={11} /> {t("close")}
        </Button>
      </Row>
    </div>
  );
}
