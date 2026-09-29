import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { BookOpen, FileText, Search, Trash2, Upload } from "lucide-react";
import { api } from "@/lib/api";
import { t, tf } from "@/lib/i18n";
import { Card, DataTable, Row, Section, td, tdStrong } from "@/components/layout";
import { EmptyState } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Textarea } from "@/components/ui/input";
import type { KnowledgeDocView, KnowledgeSearchHit } from "@/lib/types";

export function KnowledgePane() {
  const queryClient = useQueryClient();
  const docs = useQuery({ queryKey: ["knowledge"], queryFn: api.listKnowledge });
  const [text, setText] = useState("");
  const [filename, setFilename] = useState("pasted.md");
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<KnowledgeSearchHit[] | null>(null);

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

  const search = async () => setHits(await api.searchKnowledge(query, 10));

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
            <Badge tone="danger">{String(paste.error ?? fromFile.error)}</Badge>
          )}
        </Row>
      </Card>

      <Card title={t("検索")} icon={<Search size={13} />}>
        <Row className="gap-1.5">
          <Input
            className="min-w-0 flex-1"
            placeholder={t("キーワード")}
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
        {hits && (
          <ul className="m-0 flex list-none flex-col gap-2 p-0 text-xs">
            {hits.length === 0 && <li className="text-faint">{t("該当なし")}</li>}
            {hits.map((h) => (
              <li key={h.id} className="border-t border-line pt-2 first:border-0 first:pt-0">
                <Row className="gap-1.5">
                  <strong className="text-fg">{h.filename}</strong>
                  <Badge size="sm" className="font-mono">{h.score.toFixed(2)}</Badge>
                </Row>
                <p className="m-0 text-muted">{h.text.slice(0, 200)}</p>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <h2 className="text-sm font-semibold text-fg">{t("登録済みドキュメント")}</h2>
      {(docs.data ?? []).length === 0 ? (
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
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="text-danger hover:text-danger"
                  title={t("削除")}
                  aria-label={t("削除")}
                  onClick={() => { if (confirm(tf("Delete \"{name}\"?", { name: d.filename }))) remove.mutate(d.doc_id); }}
                >
                  <Trash2 size={13} />
                </Button>
              </td>
            </>
          )}
        </DataTable>
      )}
    </Section>
  );
}
