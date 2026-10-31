import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronRight,
  Download,
  FileCode,
  Sparkles,
  Upload,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { CodeBlock } from "@/components/CopyButton";
import { Card, Row, Section } from "@/components/layout";
import {
  EmptyState,
  ErrorBox,
  ListSkeleton,
  payloadBlock,
} from "@/components/shared";
import { DeleteIconButton } from "@/components/common";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";
import { Spinner } from "@/components/ui/spinner";
import { Input, Textarea } from "@/components/ui/input";
import type { SkillResourceView, SkillView } from "@/lib/types";

type Source = "paste" | "zip" | "directory";

// Raw keys stay as raw i18n keys; `t()` is applied at render time so a locale
// switch updates the tab labels without re-importing the module.
const SOURCE_LABEL: Record<Source, string> = {
  paste: "SKILL.md を貼る",
  zip: "zip ファイル",
  directory: "ディレクトリ",
};

export function SkillsPane() {
  const queryClient = useQueryClient();
  const skills = useQuery({ queryKey: ["skills"], queryFn: api.listSkills });
  const [source, setSource] = useState<Source>("paste");
  const [paste, setPaste] = useState("");
  const [directory, setDirectory] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: ["skills"] });

  const importPaste = useMutation({
    mutationFn: () =>
      api.importSkill({
        source: "paste",
        payload: paste,
        path: null,
        name: null,
      }),
    onSuccess: () => {
      setPaste("");
      setNotice(null);
      setError(null);
      void invalidate();
    },
    onError: (e) => setError(errorMessage(e)),
  });

  const importZip = useMutation({
    mutationFn: async (path: string) => {
      // Read through the Rust command surface (the fs plugin is not exposed
      // to the WebView); it returns the base64 payload importSkill expects.
      const payload = await api.readAttachment(path);
      return api.importSkill({
        source: "zip",
        payload: payload.data_base64,
        path,
        name: null,
      });
    },
    onSuccess: () => {
      setNotice(null);
      setError(null);
      void invalidate();
    },
    onError: (e) => setError(errorMessage(e)),
  });

  const importDirectory = useMutation({
    mutationFn: (path: string) =>
      api.importSkill({
        source: "directory",
        payload: null,
        path,
        name: null,
      }),
    onSuccess: () => {
      setDirectory("");
      setNotice(null);
      setError(null);
      void invalidate();
    },
    onError: (e) => setError(errorMessage(e)),
  });

  const remove = useMutation({
    mutationFn: (name: string) => api.deleteSkill(name),
    onSuccess: () => void invalidate(),
    onError: (e) => setError(errorMessage(e)),
  });

  const exportZip = useMutation({
    mutationFn: async (name: string) => {
      const target = await api.pickSaveFile({
        defaultName: `${name}.skill.zip`,
        filters: [{ name: "Skill archive", extensions: ["zip"] }],
      });
      if (!target) return;
      const b64 = await api.exportSkill(name, true);
      await api.saveBase64File(target, b64);
      setNotice(tf("{name} を {target} に書き出しました", { name, target }));
    },
    onError: (e) => setError(errorMessage(e)),
  });

  const list = skills.data ?? [];

  return (
    <Section>
      <Card title={t("取り込む")} icon={<Upload size={13} />}>
        <Segmented
          aria-label={t("取り込み元")}
          className="w-fit min-w-0 max-w-full"
        >
          {(["paste", "zip", "directory"] as Source[]).map((s) => (
            <SegmentedOption
              key={s}
              value={s}
              label={t(SOURCE_LABEL[s])}
              selected={source === s}
              onSelect={setSource}
            >
              {t(SOURCE_LABEL[s])}
            </SegmentedOption>
          ))}
        </Segmented>

        {source === "paste" && (
          <>
            <Textarea
              className="font-mono"
              rows={8}
              placeholder={
                "---\nname: my-skill        # lowercase, digits, single hyphens\ndescription: what it does  # required\n---\n\n# Instructions…"
              }
              value={paste}
              onChange={(e) => setPaste(e.target.value)}
            />
            <Row>
              <Button
                variant="primary"
                disabled={!paste.trim() || importPaste.isPending}
                onClick={() => importPaste.mutate()}
              >
                <Upload size={12} /> {t("取り込む")}
              </Button>
            </Row>
          </>
        )}

        {source === "zip" && (
          <Row>
            <Button
              variant="primary"
              onClick={async () => {
                const picked = await api.pickOpenFile({
                  filters: [{ name: "Skill archive", extensions: ["zip"] }],
                });
                if (picked.length > 0) importZip.mutate(picked[0]);
              }}
            >
              <Upload size={12} /> {t(".skill.zip を選択")}
            </Button>
            {importZip.isPending && (
              <span className="inline-flex items-center gap-1.5 text-xs text-muted">
                <Spinner size={12} /> {t("読み込み中…")}
              </span>
            )}
          </Row>
        )}

        {source === "directory" && (
          <Row className="gap-1.5">
            <Input
              className="min-w-0 flex-1 font-mono"
              placeholder="/path/to/skill-directory"
              value={directory}
              onChange={(e) => setDirectory(e.target.value)}
            />
            <Button
              onClick={async () => {
                const picked = await api.pickFolder();
                if (picked) setDirectory(picked);
              }}
            >
              {t("選択")}
            </Button>
            <Button
              variant="primary"
              disabled={!directory || importDirectory.isPending}
              onClick={() => importDirectory.mutate(directory)}
            >
              <Upload size={12} /> {t("取り込む")}
            </Button>
          </Row>
        )}

        {notice && <Alert tone="success">{notice}</Alert>}
        {error && <ErrorBox error={error} />}
      </Card>

      {skills.isPending ? (
        <ListSkeleton />
      ) : skills.isError ? (
        <ErrorBox error={skills.error} />
      ) : list.length === 0 ? (
        <EmptyState
          icon={<Sparkles size={22} />}
          title={t("まだ Skill はありません。")}
        />
      ) : (
        list.map((s: SkillView) => (
          <Card key={s.name}>
            <Row align="between">
              <strong className="flex items-center gap-1.5 text-sm font-medium text-fg">
                <Sparkles size={12} className="text-faint" /> {s.name}
              </strong>
              <Row className="flex-nowrap gap-1">
                <Button
                  size="sm"
                  onClick={() =>
                    setSelected(selected === s.name ? null : s.name)
                  }
                >
                  <FileCode size={12} /> {t("リソース")}
                </Button>
                <Button
                  size="icon-sm"
                  title={t("エクスポート")}
                  aria-label={t("エクスポート")}
                  onClick={() => exportZip.mutate(s.name)}
                >
                  <Download size={13} />
                </Button>
                <DeleteIconButton
                  title={t("Skill を削除しますか？")}
                  description={s.name}
                  pending={remove.isPending}
                  onDelete={() => remove.mutate(s.name)}
                />
              </Row>
            </Row>
            <p className="m-0 text-xs text-muted">{s.description}</p>
            <pre className={`${payloadBlock} max-h-60`}>{s.body_markdown}</pre>
            {selected === s.name && <ResourceList name={s.name} />}
          </Card>
        ))
      )}
    </Section>
  );
}

function ResourceList({ name }: { name: string }) {
  const [resources, setResources] = useState<SkillResourceView[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  if (error) return <ErrorBox error={error} />;

  if (resources === null) {
    return (
      <Row>
        <Button
          size="sm"
          onClick={async () => {
            try {
              setResources(await api.skillResources(name));
            } catch (e) {
              setError(errorMessage(e));
            }
          }}
        >
          {t("同梱リソースを読み込む")}
        </Button>
      </Row>
    );
  }

  return (
    <div className="flex flex-col gap-1.5">
      {resources.length === 0 && (
        <p className="m-0 text-xs text-faint">
          {t("同梱リソースはありません。")}
        </p>
      )}
      {resources.map((r) => (
        <details key={r.path} className="group">
          <summary className="flex cursor-pointer list-none items-center gap-1 font-mono text-xs text-muted transition-colors hover:text-fg">
            <ChevronRight
              size={12}
              aria-hidden="true"
              className="shrink-0 transition-transform group-open:rotate-90"
            />
            {r.path}
          </summary>
          <CodeBlock language="" code={r.content} />
        </details>
      ))}
    </div>
  );
}
