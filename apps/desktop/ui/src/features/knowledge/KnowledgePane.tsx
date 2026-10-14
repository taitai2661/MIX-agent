import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { BookOpen, FileText, Search, Upload } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { useSearch } from "@/lib/useSearch";
import { Card, DataTable, Row, Section, td, tdStrong } from "@/components/layout";
import { EmptyState, ErrorBadge, ErrorBox, TableSkeleton } from "@/components/shared";
import { DeleteIconButton, SearchEmptyState, SearchForm, SectionHeading } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Textarea } from "@/components/ui/input";
import type { KnowledgeDocView } from "@/lib/types";

export function KnowledgePane() {
  const queryClient = useQueryClient();
  const docs = useQuery({ queryKey: ["knowledge"], queryFn: api.listKnowledge });
  const [text, setText] = useState("");
  const [filename, setFilename] = useState("pasted.md");
  const search = useSearch((q, n) => api.searchKnowledge(q, n), 10);

  const paste = useMutation({
    mutationFn: () => api.ingestKnowledge({ filename, text }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["knowledge"] });
      setText("");
    },
  });

  const fromFile = useMutation({
    mutationFn: (path: string) => api.importKnowledgeFile({ path }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["knowledge"] }),
  });

  const remove = useMutation({
    mutationFn: (docId: string) => api.deleteKnowledge(docId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["knowledge"] }),
  });

  // Same as the memory pane: a failed search used to reject into nothing, so
  // the results list just stayed empty with no explanation.
  // State lives in `useSearch` now; this keeps the shared submit handler name.
  const runSearch = () => void search.run();

  return (
    <Section>
      <Card title={t("取り込む")} icon={<Upload size={13} />}>
        <Row className="gap-1.5">
          <Input
            className="min-w-0 flex-1"
            placeholder={t("ファイル名 (例: architecture.md)")}
            value={filename}
            onChange={(e) => setFilename(e.target.value)}
          />
          <Button
            onClick={async () => {
              const picked = await open({
                multiple: false,
                filters: [{ name: "Text", extensions: ["md", "txt", "json", "yaml", "yml"] }],
              });
              if (typeof picked === "string") fromFile.mutate(picked);
            }}
          >
            <Upload size={12} /> {t("ファイルから")}
          </Button>
        </Row>
        <Textarea
          rows={6}
          placeholder={t("ここに本文を貼り付け…")}
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        <Row>
          <Button
            variant="primary"
            disabled={!text.trim() || paste.isPending}
            onClick={() => paste.mutate()}
          >
            <FileText size={12} /> {t("追加する")}
          </Button>
          {(paste.isError || fromFile.isError) && (
            <ErrorBadge error={paste.error ?? fromFile.error} />
          )}
        </Row>
      </Card>

      <Card title={t("検索")} icon={<Search size={13} />}>
        <SearchForm
          value={search.query}
          onChange={search.setQuery}
          onSubmit={runSearch}
          searching={search.searching}
          placeholder={t("キーワード")}
        />
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
                  <strong className="text-sm font-medium text-fg">{h.filename}</strong>
                  <Badge size="sm" className="font-mono">{h.score.toFixed(2)}</Badge>
                </Row>
                <p className="m-0 text-muted">{h.text.slice(0, 200)}</p>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <SectionHeading>{t("登録済みドキュメント")}</SectionHeading>
      {docs.isPending ? (
        <TableSkeleton />
      ) : docs.isError ? (
        <ErrorBox error={docs.error} />
      ) : (docs.data ?? []).length === 0 ? (
        <EmptyState icon={<BookOpen size={22} />} title={t("まだドキュメントがありません。")} />
      ) : (
        <DataTable
          columns={[t("ファイル名"), t("チャンク"), t("登録日時"), ""]}
          rows={docs.data ?? []}
          rowKey={(d: KnowledgeDocView) => d.doc_id}
        >
          {(d) => (
            <>
              <td className={tdStrong}>{d.filename}</td>
              <td className={td}>{d.chunk_count}</td>
              <td className={td}>{new Date(d.created_at).toLocaleString()}</td>
              <td className={`${td} w-12`}>
                <DeleteIconButton
                  title={t("ドキュメントを削除しますか？")}
                  description={d.filename}
                  pending={remove.isPending}
                  onDelete={() => remove.mutate(d.doc_id)}
                />
              </td>
            </>
          )}
        </DataTable>
      )}
    </Section>
  );
}
