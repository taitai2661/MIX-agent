import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Download, FileCode, Sparkles, Trash2, Upload } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { CodeBlock } from "@/components/CopyButton";
import { Card, Row, Section } from "@/components/layout";
import { EmptyState } from "@/components/shared";
import { Alert } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input, Textarea } from "@/components/ui/input";
import type { SkillResourceView, SkillView } from "@/lib/types";

type Source = "paste" | "zip" | "directory";

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

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ["skills"] });

  const importPaste = useMutation({
    mutationFn: () => api.importSkill({
      source: "paste", payload: paste, path: null, name: null,
    }),
    onSuccess: () => { setPaste(""); setNotice(null); void invalidate(); },
    onError: (e) => setError(String(e)),
  });

  const importZip = useMutation({
    mutationFn: async (path: string) => {
      // Read through the Rust command surface (the fs plugin is not exposed
      // to the WebView); it returns the base64 payload importSkill expects.
      const payload = await api.readAttachment(path);
      return api.importSkill({
        source: "zip", payload: payload.data_base64, path, name: null,
      });
    },
    onSuccess: () => { setNotice(null); void invalidate(); },
    onError: (e) => setError(String(e)),
  });

  const importDirectory = useMutation({
    mutationFn: (path: string) => api.importSkill({
      source: "directory", payload: null, path, name: null,
    }),
    onSuccess: () => { setDirectory(""); setNotice(null); void invalidate(); },
    onError: (e) => setError(String(e)),
  });

  const remove = useMutation({
    mutationFn: (name: string) => api.deleteSkill(name),
    onSuccess: () => void invalidate(),
    onError: (e) => setError(String(e)),
  });

  const exportZip = useMutation({
    mutationFn: async (name: string) => {
      const target = await save({
        defaultPath: `${name}.skill.zip`,
        filters: [{ name: "Skill archive", extensions: ["zip"] }],
      });
      if (!target) return;
      const b64 = await api.exportSkill(name, true);
      await api.saveBase64File(target, b64);
      setNotice(`Exported ${name} to ${target}`);
    },
    onError: (e) => setError(String(e)),
  });

  const list = skills.data ?? [];

  return (
    <Section>
      <Card title="取り込む" icon={<Upload size={13} />}>
        <div
          role="tablist"
          aria-label="取り込み元"
          className="inline-flex w-fit items-center gap-0.5 rounded-lg bg-inset p-0.5"
        >
          {(["paste", "zip", "directory"] as Source[]).map((s) => (
            <button
              key={s}
              type="button"
              role="tab"
              aria-selected={source === s}
              onClick={() => setSource(s)}
              className={cn(
                "h-6 rounded-md px-2 text-xs font-medium",
                "text-muted transition-colors duration-100 hover:text-fg",
                source === s && "bg-surface text-fg shadow-xs",
              )}
            >
              {SOURCE_LABEL[s]}
            </button>
          ))}
        </div>

        {source === "paste" && (
          <>
            <Textarea
              className="font-mono"
              rows={8}
              placeholder={"---\nname: my-skill\ndescription: what it does\n---\n\n# Instructions…"}
              value={paste}
              onChange={(e) => setPaste(e.target.value)}
            />
            <Row>
              <Button
                variant="primary"
                disabled={!paste.trim() || importPaste.isPending}
                onClick={() => importPaste.mutate()}
              >
                <Upload size={12} /> 取り込む
              </Button>
            </Row>
          </>
        )}

        {source === "zip" && (
          <Row>
            <Button
              variant="primary"
              onClick={async () => {
                const picked = await open({
                  multiple: false,
                  filters: [{ name: "Skill archive", extensions: ["zip"] }],
                });
                if (typeof picked === "string") importZip.mutate(picked);
              }}
            >
              <Upload size={12} /> .skill.zip を選択
            </Button>
            {importZip.isPending && <span className="text-xs text-muted">読み込み中…</span>}
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
                const picked = await open({ directory: true, multiple: false });
                if (typeof picked === "string") setDirectory(picked);
              }}
            >
              選択
            </Button>
            <Button
              variant="primary"
              disabled={!directory || importDirectory.isPending}
              onClick={() => importDirectory.mutate(directory)}
            >
              <Upload size={12} /> 取り込む
            </Button>
          </Row>
        )}

        {notice && <Alert tone="success">{notice}</Alert>}
        {error && <Alert tone="danger">{error}</Alert>}
      </Card>

      {list.length === 0 ? (
        <EmptyState icon={<Sparkles size={22} />} title="まだ Skill はありません。" />
      ) : (
        list.map((s: SkillView) => (
          <Card key={s.name}>
            <Row align="between">
              <strong className="flex items-center gap-1.5 text-sm text-fg">
                <Sparkles size={12} className="text-faint" /> {s.name}
              </strong>
              <Row className="flex-nowrap gap-1">
                <Button
                  size="sm"
                  onClick={() => setSelected(selected === s.name ? null : s.name)}
                >
                  <FileCode size={12} /> resources
                </Button>
                <Button
                  size="sm"
                  title="Export as .skill.zip"
                  aria-label="Export as .skill.zip"
                  onClick={() => exportZip.mutate(s.name)}
                >
                  <Download size={12} />
                </Button>
                <Button
                  size="sm"
                  className="text-danger hover:text-danger"
                  title={t("削除")}
                  aria-label={t("削除")}
                  onClick={() => { if (confirm(`Delete skill "${s.name}"?`)) remove.mutate(s.name); }}
                >
                  <Trash2 size={12} />
                </Button>
              </Row>
            </Row>
            <p className="m-0 text-xs text-muted">{s.description}</p>
            <pre className="m-0 max-h-60 overflow-auto whitespace-pre-wrap rounded-md bg-inset p-2 font-mono text-2xs leading-4 text-muted">
              {s.body_markdown}
            </pre>
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

  if (error) return <Alert tone="danger">{error}</Alert>;

  if (resources === null) {
    return (
      <Row>
        <Button
          size="sm"
          onClick={async () => {
            try {
              setResources(await api.skillResources(name));
            } catch (e) {
              setError(String(e));
            }
          }}
        >
          Load bundled resources
        </Button>
      </Row>
    );
  }

  return (
    <div className="flex flex-col gap-1.5">
      {resources.length === 0 && (
        <p className="m-0 text-xs text-faint">No bundled resources.</p>
      )}
      {resources.map((r) => (
        <details key={r.path} className="group">
          <summary className="cursor-pointer list-none font-mono text-xs text-muted transition-colors hover:text-fg">
            {r.path}
          </summary>
          <CodeBlock language="" code={r.content} />
        </details>
      ))}
    </div>
  );
}
