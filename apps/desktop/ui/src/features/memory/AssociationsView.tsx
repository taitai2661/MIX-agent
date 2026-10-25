import { useQuery } from "@tanstack/react-query";
import { Link2, X } from "lucide-react";
import { api } from "@/lib/api";
import { t, tf } from "@/lib/i18n";
import { Card, Row, SectionLabel } from "@/components/layout";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { EmptyState, ErrorBox, ListSkeleton } from "@/components/shared";
import type { MemoryLinkView, MemoryView } from "@/lib/types";

/**
 * §4's `memory_links`, read back the way §4 writes them.
 *
 * The one thing this screen must not get wrong is **direction**: §4 fixes
 * `target_memory_id` as *the row that carries the §3 column* and
 * `source_memory_id` as *the Memory that column names*. Rendered the other way
 * round, `supersedes` would read "the old row replaced the new one" — the
 * statement §4's own justification rules out — so the two halves sit under
 * separate headings rather than in one flat list, and every endpoint keeps its
 * `source` / `target` label instead of being inferred from position.
 *
 * `kind` is §2's vocabulary and is never translated: a gloss would be a second
 * name for a word the spec owns.
 */
export function AssociationsView({
  subject,
  titles,
  onClose,
  onOpen,
}: {
  /** The Memory whose edges are on screen. */
  subject: MemoryView;
  /** `id → title` for the rows the pane already loaded, used as link labels. */
  titles: Record<string, string>;
  onClose: () => void;
  /** Open another Memory's edges — only offered for a row we hold a title for. */
  onOpen: (id: string) => void;
}) {
  const links = useQuery({
    queryKey: ["memory_associations", subject.id],
    queryFn: () => api.memoryAssociations(subject.id),
  });

  const rows = links.data ?? [];
  // §4's rule applied literally: an edge whose `target` is this row is one
  // where this row carries the column, so the *other* end is what the column
  // names.
  const carries = rows.filter((link) => link.target_memory_id === subject.id);
  const named = rows.filter((link) => link.source_memory_id === subject.id);

  return (
    <Card
      title={subject.title || t("無題の Memory")}
      icon={<Link2 size={13} />}
      action={
        <Button
          size="icon-sm"
          variant="ghost"
          title={t("閉じる")}
          aria-label={t("閉じる")}
          onClick={onClose}
        >
          <X size={13} />
        </Button>
      }
    >
      <Row className="gap-1.5">
        <Badge size="sm" className="font-mono">
          {subject.role}
        </Badge>
        <Badge size="sm" className="font-mono">
          {subject.scope}
        </Badge>
        <Badge size="sm">{subject.lifecycle_state}</Badge>
        <span className="text-xs text-muted">
          {t("§4 の向き: target は列を持つ行、source はその列が指す Memory。")}
        </span>
      </Row>

      {links.isPending ? (
        <ListSkeleton rows={2} />
      ) : links.isError ? (
        <ErrorBox error={links.error} />
      ) : rows.length === 0 ? (
        <EmptyState
          icon={<Link2 size={22} />}
          title={t("リンクはまだありません。")}
          sub={t(
            "想起のたびに関連する Memory と結び、上書き・矛盾の鏡は §3 の遷移が書き込みます。",
          )}
        />
      ) : (
        <>
          <EdgeGroup
            heading={t("この Memory が終点（target）— 列を持つ行")}
            note={t("この行の §3 の列が相手を名指しています。")}
            links={carries}
            subject={subject}
            titles={titles}
            onOpen={onOpen}
          />
          <EdgeGroup
            heading={t("この Memory が始点（source）— 列が指す側")}
            note={t("相手の行の §3 の列がこの Memory を名指しています。")}
            links={named}
            subject={subject}
            titles={titles}
            onOpen={onOpen}
          />
        </>
      )}
    </Card>
  );
}

function EdgeGroup({
  heading,
  note,
  links,
  subject,
  titles,
  onOpen,
}: {
  heading: string;
  note: string;
  links: MemoryLinkView[];
  subject: MemoryView;
  titles: Record<string, string>;
  onOpen: (id: string) => void;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <SectionLabel
        className="px-0 pb-0"
        trailing={
          <Badge size="sm" tone="neutral" className="font-mono">
            {tf("{n} 件", { n: links.length })}
          </Badge>
        }
      >
        {heading}
      </SectionLabel>
      <p className="m-0 text-xs text-muted">{note}</p>
      {links.length === 0 ? (
        <p className="m-0 text-xs text-faint">{t("該当なし")}</p>
      ) : (
        <ul className="m-0 flex list-none flex-col gap-2 p-0">
          {links.map((link) => (
            <li
              key={link.id}
              className="flex flex-col gap-1 border-t border-line pt-2 first:border-0 first:pt-0"
            >
              <Row className="gap-1.5">
                <Endpoint
                  id={link.source_memory_id}
                  side="source"
                  subject={subject}
                  titles={titles}
                  onOpen={onOpen}
                />
                <span
                  className="shrink-0 font-mono text-2xs text-fg"
                  title={t("関係の種類（§2 relation_kinds）")}
                >
                  {link.kind}
                </span>
                <span aria-hidden className="shrink-0 text-2xs text-faint">
                  →
                </span>
                <Endpoint
                  id={link.target_memory_id}
                  side="target"
                  subject={subject}
                  titles={titles}
                  onOpen={onOpen}
                />
              </Row>
              <Row className="gap-2 text-2xs text-faint">
                <span className="font-mono">
                  {tf("weight {w}", { w: link.weight.toFixed(2) })}
                </span>
                <span className="font-mono">
                  {tf("coactivation {n}", { n: link.coactivation_count })}
                </span>
                <span>{new Date(link.updated_at).toLocaleString()}</span>
              </Row>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** One end of an edge: §4's role badge plus a label you can follow. */
function Endpoint({
  id,
  side,
  subject,
  titles,
  onOpen,
}: {
  id: string;
  side: "source" | "target";
  subject: MemoryView;
  titles: Record<string, string>;
  onOpen: (id: string) => void;
}) {
  const isSubject = id === subject.id;
  const known = Object.prototype.hasOwnProperty.call(titles, id);
  const label = titles[id] ?? `${id.slice(0, 8)}…`;
  return (
    <span className="inline-flex min-w-0 items-center gap-1">
      <Badge size="sm" tone={side === "target" ? "warning" : "info"}>
        {side}
      </Badge>
      {known ? (
        <button
          type="button"
          className="max-w-[16rem] truncate border-0 bg-transparent p-0 text-start text-xs text-brand underline-offset-2 hover:underline"
          title={isSubject ? id : tf("{name} のリンクを表示", { name: label })}
          onClick={() => onOpen(id)}
        >
          {isSubject ? t("この Memory") : label}
        </button>
      ) : (
        <span
          className="max-w-[16rem] truncate font-mono text-xs text-muted"
          title={id}
        >
          {label}
        </span>
      )}
    </span>
  );
}
