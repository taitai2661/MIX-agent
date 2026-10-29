import { t } from "@/app/i18n";
import { api } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox } from "@/components/shared";
import { useQuery } from "@tanstack/react-query";
import { Link2 } from "lucide-react";

/** One §4 edge, exactly as `GET /memories/{key}/associations` returns it. */
export type AssociationEdge = {
  id?: string;
  source_memory_id: string;
  target_memory_id: string;
  kind: string;
  weight?: number;
  coactivation_count?: number;
  created_at?: string | null;
  updated_at?: string | null;
};

function short(id: string): string {
  return id ? id.slice(0, 8) : "";
}

/**
 * §4's edges for one memory, in §4's own shape and direction.
 *
 * `target_memory_id` is the row that *carries* the §3 column
 * (`superseded_by_id` / `disputed_by_id`); `source_memory_id` is the id that
 * column points at. Drawing the arrow the other way round would read "the old
 * row replaced the new one", the exact inversion §1/§3 forbid, so every row
 * spells out which end holds the column and which end this memory is.
 *
 * `kind` and the column names are shown verbatim: they are §2/§4 vocabulary,
 * and a translated label would no longer match what the server stored.
 */
export function AssociationList({
  memoryId,
  edges,
}: {
  memoryId: string;
  edges: AssociationEdge[];
}) {
  if (!edges.length) {
    return (
      <p className="memory-network-empty">
        {t("この記憶に繋がりはまだありません。")}
      </p>
    );
  }
  return (
    <div className="memory-connections">
      <p className="memory-network-empty">
        {t("target が列を持つ行、source がその列が指す Memory。")}
      </p>
      <ol className="memory-network-list">
        {edges.map((edge, index) => (
          <li
            className="memory-network-edge"
            key={
              edge.id ||
              `${edge.source_memory_id}-${edge.target_memory_id}-${index}`
            }
          >
            <span className="memory-network-link">
              <span className="memory-network-line" aria-hidden="true" />
              <code>{edge.kind}</code>
            </span>
            <div className="memory-network-node">
              <span>
                <code>{short(edge.source_memory_id)}</code> →{" "}
                <code>{short(edge.target_memory_id)}</code>
              </span>
              <small>
                source {t("（列が指す側）")}
                {edge.source_memory_id === memoryId
                  ? ` · ${t("この Memory")}`
                  : ""}{" "}
                / target {t("（列を持つ側）")}
                {edge.target_memory_id === memoryId
                  ? ` · ${t("この Memory")}`
                  : ""}
              </small>
              <small>
                weight {Number(edge.weight ?? 0).toFixed(2)} ·
                coactivation_count {Number(edge.coactivation_count ?? 0)}
              </small>
            </div>
          </li>
        ))}
      </ol>
    </div>
  );
}

/** Fetching wrapper: both directions of one memory's §4 edges (owner-scoped). */
export function AssociationsView({
  memoryId,
  onClose,
}: {
  memoryId: string;
  onClose?: () => void;
}) {
  const links = useQuery<AssociationEdge[]>({
    queryKey: ["/memories", memoryId, "associations"],
    queryFn: () =>
      api(`/memories/${encodeURIComponent(memoryId)}/associations`),
  });
  return (
    <div className="memory-detail">
      <div className="memory-detail-header">
        <h4>
          <Link2 size={13} /> {t("関連")}
          {links.data ? ` (${links.data.length})` : ""}
        </h4>
        {onClose && (
          <Button variant="ghost" onClick={onClose}>
            {t("閉じる")}
          </Button>
        )}
      </div>
      <ErrorBox error={links.error} />
      {links.isFetching && !links.data ? (
        <p className="memory-network-empty">{t("読み込み中…")}</p>
      ) : null}
      {links.data ? (
        <AssociationList memoryId={memoryId} edges={links.data} />
      ) : null}
    </div>
  );
}
