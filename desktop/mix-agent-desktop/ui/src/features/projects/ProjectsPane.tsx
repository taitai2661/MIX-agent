import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  ChevronLeft, FileText, Folder, FolderOpen, MessageSquare, Plus, Trash2,
} from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { relativeTime } from "@/lib/format";
import { CONVERSATIONS_QUERY_KEY, PROJECTS_QUERY_KEY, useConversations, useProjects } from "@/lib/queries";
import { useUi } from "@/lib/ui";
import { Card, Field, ListRow, Page, Row } from "@/components/layout";
import { EmptyState, ErrorBadge, ListSkeleton } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import { Input, Textarea } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { ProjectView } from "@/lib/types";

const NAME_MAX = 100;
const INSTRUCTIONS_MAX = 20000;
const SOURCES_MAX = 20;
const SOURCE_MAX = 20000;

export function ProjectsPane() {
  const queryClient = useQueryClient();
  const setConversationId = useUi((s) => s.setConversationId);
  const setView = useUi((s) => s.setView);
  const setActiveProjectId = useUi((s) => s.setActiveProjectId);
  const mode = useUi((s) => s.mode);
  const modelId = useUi((s) => s.modelId);
  const { confirm, dialog } = useConfirm();

  const projects = useProjects();
  const conversations = useConversations();

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [draftRoot, setDraftRoot] = useState<string | null>(null);
  const [nameDraft, setNameDraft] = useState<string | null>(null);
  const [instructionsDraft, setInstructionsDraft] = useState<string | null>(null);
  const [sourceDraft, setSourceDraft] = useState("");

  const list = projects.data ?? [];
  const selected = list.find((p) => p.id === selectedId) ?? null;
  const linked = selected
    ? (conversations.data ?? []).filter((c) => c.project_id === selected.id)
    : [];

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
    void queryClient.invalidateQueries({ queryKey: CONVERSATIONS_QUERY_KEY });
  };

  const create = useMutation({
    mutationFn: () =>
      api.createProject({
        name: draftName.trim(),
        instructions: "",
        // Folder-first: the backend rejects a project without a directory.
        rootPath: draftRoot!,
        sources: [],
      }),
    onSuccess: (created) => {
      invalidate();
      setDraftName("");
      setDraftRoot(null);
      setActiveProjectId(created.id);
      setSelectedId(created.id);
      setNameDraft(created.name);
      setInstructionsDraft(created.instructions);
    },
  });

  const update = useMutation({
    mutationFn: (input: Parameters<typeof api.updateProject>[1]) =>
      api.updateProject(selected!.id, input),
    onSuccess: () => invalidate(),
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteProject(id),
    onSuccess: (_data, id) => {
      invalidate();
      if (selectedId === id) setSelectedId(null);
      setActiveProjectId(null);
    },
  });

  const pickFolder = async () => {
    const path = await openDialog({ directory: true, multiple: false });
    if (typeof path === "string" && path) {
      update.mutate({ rootPath: path });
    }
  };

  const pickDraftFolder = async () => {
    const path = await openDialog({ directory: true, multiple: false });
    if (typeof path === "string" && path) setDraftRoot(path);
  };

  const newChatInProject = async () => {
    if (!selected) return;
    const created = await api.createConversation({
      mode, model_id: modelId, title: null, workspace_id: null, project_id: selected.id,
    });
    invalidate();
    setActiveProjectId(selected.id);
    setConversationId(created.id);
    setView("chat");
  };

  const openDetail = (project: ProjectView) => {
    setSelectedId(project.id);
    setNameDraft(project.name);
    setInstructionsDraft(project.instructions);
    setActiveProjectId(project.id);
    setSourceDraft("");
  };

  if (selected && nameDraft !== null && instructionsDraft !== null) {
    return (
      <Page
        title={selected.name}
        sub={selected.rootPath}
        actions={
          <Button size="sm" onClick={() => setSelectedId(null)}>
            <ChevronLeft size={13} /> {t("戻る")}
          </Button>
        }
      >
        <Card title={t("名前")} icon={<Folder size={13} />}>
          <Input
            maxLength={NAME_MAX}
            value={nameDraft}
            onChange={(e) => setNameDraft(e.target.value)}
            onBlur={() => {
              const next = nameDraft.trim();
              if (next && next !== selected.name) update.mutate({ name: next });
            }}
          />
        </Card>

        <Card title={t("フォルダ")} icon={<FolderOpen size={13} />}>
          <Row>
            <code className="min-w-0 flex-1 truncate font-mono text-xs text-muted">
              {selected.rootPath}
            </code>
            <Button size="sm" onClick={() => void pickFolder()}>
              {t("フォルダを選択")}
            </Button>
          </Row>
          <p className="m-0 text-xs text-faint">
            {t("このフォルダがエージェントの作業ディレクトリになります")}
          </p>
          <p className="m-0 text-xs text-faint">
            <FileText size={11} className="me-0.5 inline-block align-[-1.5px]" /> AGENTS.md /{" "}
            CLAUDE.md {t("を検出すると自動で読み込みます")}
          </p>
        </Card>

        <Card title={t("指示")}>
          <Textarea
            maxLength={INSTRUCTIONS_MAX}
            value={instructionsDraft}
            onChange={(e) => setInstructionsDraft(e.target.value)}
            placeholder={t("プロジェクトへの指示…")}
          />
          <Row>
            <Button
              variant="primary"
              size="sm"
              disabled={instructionsDraft === selected.instructions}
              onClick={() => update.mutate({ instructions: instructionsDraft })}
            >
              {t("指示を保存")}
            </Button>
            <span className="text-xs text-faint">
              {instructionsDraft.length} / {INSTRUCTIONS_MAX}
            </span>
          </Row>
        </Card>

        <Card title={t("資料")}>
          {selected.sources.length > 0 && (
            <ul className="flex flex-col gap-1">
              {selected.sources.map((item, index) => (
                <li key={index}>
                  <Row>
                    <span className="min-w-0 flex-1 truncate text-xs text-muted">
                      {item.slice(0, 120)}
                    </span>
                    <Button
                      variant="ghost-danger"
                      size="icon-sm"
                      title={t("削除")}
                      aria-label={t("削除")}
                      onClick={async () => {
                        const ok = await confirm({
                          title: t("資料を削除しますか？"),
                          description: item.slice(0, 120),
                          confirmLabel: t("削除"),
                          tone: "danger",
                        });
                        if (ok) {
                          update.mutate({
                            sources: selected.sources.filter((_, i) => i !== index),
                          });
                        }
                      }}
                    >
                      <Trash2 size={12} />
                    </Button>
                  </Row>
                </li>
              ))}
            </ul>
          )}
          <Textarea
            className="min-h-18"
            maxLength={SOURCE_MAX}
            value={sourceDraft}
            onChange={(e) => setSourceDraft(e.target.value)}
            placeholder={t("資料テキストを貼り付け…")}
          />
          <Row>
            <Button
              size="sm"
              disabled={
                !sourceDraft.trim()
                || selected.sources.length >= SOURCES_MAX
                || update.isPending
              }
              onClick={() => {
                update.mutate({ sources: [...selected.sources, sourceDraft.trim()] });
                setSourceDraft("");
              }}
            >
              <Plus size={12} /> {t("資料を追加")}
            </Button>
            <span className="text-xs text-faint">
              {selected.sources.length} / {SOURCES_MAX}
            </span>
          </Row>
        </Card>

        <Card title={t("紐づく会話")} icon={<MessageSquare size={13} />}>
          <Row>
            <Button variant="primary" size="sm" onClick={() => void newChatInProject()}>
              <Plus size={12} /> {t("このプロジェクトで新しいチャット")}
            </Button>
          </Row>
          {linked.length === 0 ? (
            <p className="m-0 text-xs text-faint">{t("会話がありません")}</p>
          ) : (
            <ul className="flex flex-col gap-0.5">
              {linked.map((c) => (
                <li key={c.id}>
                  <ListRow
                    className="h-auto flex-col items-start gap-0.5 py-1.5"
                    onClick={() => {
                      setConversationId(c.id);
                      setView("chat");
                    }}
                  >
                    <span className="w-full truncate text-sm font-medium text-fg">{c.title}</span>
                    <span className="text-xs text-faint">{relativeTime(c.updated_at)}</span>
                  </ListRow>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Row>
          <Button
            size="sm"
            variant="danger"
            disabled={remove.isPending}
            onClick={async () => {
              if (await confirm({
                title: t("プロジェクトを削除しますか？"),
                description: t("会話は残ります。"),
                confirmLabel: t("削除"),
                tone: "danger",
              })) {
                remove.mutate(selected.id);
              }
            }}
          >
            <Trash2 size={12} /> {t("削除")}
          </Button>
          {update.isError && <ErrorBadge error={update.error} />}
          {create.isError && <ErrorBadge error={create.error} />}
        </Row>
        {dialog}
      </Page>
    );
  }

  return (
    <Page title={t("プロジェクト")} sub={t("プロジェクト単位の資料と指示を管理します。")}>
      <Card title={t("作成")} icon={<Folder size={13} />}>
        {/* Not wrapped in `Field`: its `<label>` would swallow the picker
            button's accessible name. */}
        <div className="flex flex-col gap-1.5">
          <Label>{t("フォルダ")}</Label>
          <Row>
            <code className="min-w-0 flex-1 truncate font-mono text-xs text-muted">
              {draftRoot ?? t("未設定")}
            </code>
            <Button size="sm" onClick={() => void pickDraftFolder()}>
              {t("フォルダを選択")}
            </Button>
          </Row>
          <span className="text-xs text-faint">
            {t("このフォルダがエージェントの作業ディレクトリになります")}
          </span>
        </div>
        <Field label={t("名前")} hint={t("未入力の場合はフォルダ名が使われます")}>
          <Input
            maxLength={NAME_MAX}
            placeholder={t("Project 名")}
            value={draftName}
            onChange={(e) => setDraftName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && draftRoot) create.mutate();
            }}
          />
        </Field>
        <Row>
          <Button
            variant="primary"
            disabled={!draftRoot || create.isPending}
            onClick={() => create.mutate()}
          >
            <Folder size={12} /> {t("作成")}
          </Button>
          {create.isError && <ErrorBadge error={create.error} />}
        </Row>
      </Card>

      {projects.isPending ? (
        <ListSkeleton />
      ) : list.length === 0 ? (
        <EmptyState icon={<Folder size={22} />} title={t("プロジェクト")} sub={t("まだありません。")} />
      ) : (
        <div className="flex flex-col gap-3">
          {list.map((p) => (
            <Card key={p.id}>
              <Row align="between">
                <button
                  type="button"
                  onClick={() => openDetail(p)}
                  className="flex min-w-0 flex-1 items-baseline gap-2 text-start text-sm font-semibold text-fg"
                >
                  <span className="truncate">{p.name}</span>
                  {p.rootPath && (
                    <span className="truncate text-xs font-normal text-faint">{p.rootPath}</span>
                  )}
                </button>
                <Row className="flex-nowrap gap-2">
                  <span className="text-xs text-faint">{relativeTime(p.updated_at)}</span>
                  <Button
                    variant="ghost-danger"
                    size="icon-sm"
                    title={t("削除")}
                    aria-label={t("削除")}
                    onClick={async () => {
                      if (await confirm({
                        title: t("プロジェクトを削除しますか？"),
                        description: t("会話は残ります。"),
                        confirmLabel: t("削除"),
                        tone: "danger",
                      })) {
                        remove.mutate(p.id);
                      }
                    }}
                  >
                    <Trash2 size={13} />
                  </Button>
                </Row>
              </Row>
              {p.instructions && (
                <p className="m-0 whitespace-pre-wrap text-xs text-muted">{p.instructions}</p>
              )}
            </Card>
          ))}
        </div>
      )}
      {dialog}
    </Page>
  );
}
