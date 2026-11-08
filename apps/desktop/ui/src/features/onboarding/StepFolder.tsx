import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Folder } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { PROJECTS_QUERY_KEY, useProjects } from "@/lib/queries";
import { useUi } from "@/lib/ui";
import { Field, Row } from "@/components/layout";
import { ErrorBox } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";

/**
 * Step 2: pick the folder the agent is allowed to work in.
 *
 * Skippable, because a projectless chat is a legitimate way to use the app and
 * forcing a folder here would be the first thing that felt like setup work. But
 * the copy is explicit about what a project buys, since without one the tools
 * fall back to the process working directory — which for a Dock-launched app is
 * `/`.
 */
export function StepFolder({
  onDone,
  onSkip,
}: {
  onDone: (projectName: string | null) => void;
  onSkip: () => void;
}) {
  const queryClient = useQueryClient();
  const projects = useProjects();
  const setActiveProjectId = useUi((s) => s.setActiveProjectId);
  const [root, setRoot] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);

  const create = useMutation({
    mutationFn: async () => {
      if (!root) throw new Error(t("フォルダを選択してください"));
      return api.createProject({
        name: name.trim(),
        instructions: "",
        rootPath: root,
        sources: [],
      });
    },
    onSuccess: (created) => {
      void queryClient.invalidateQueries({ queryKey: PROJECTS_QUERY_KEY });
      setActiveProjectId(created.id);
      onDone(created.name);
    },
    onError: (err) => setError(errorMessage(err)),
  });

  const pickFolder = async () => {
    // Cancelling the picker leaves the step as it was rather than clearing it —
    // the dialog returning `null` is a "never mind", not a "clear it".
    const path = await api.pickFolder();
    if (path) {
      setRoot(path);
      setError(null);
    }
  };

  const existing = projects.data?.[0] ?? null;

  return (
    <div className="flex flex-col gap-5">
      {existing ? (
        <div className="flex items-center gap-3 rounded-xl border border-line bg-inset/50 p-3.5">
          <span className="grid size-9 shrink-0 place-items-center rounded-lg bg-brand-soft text-brand">
            <Folder size={16} aria-hidden="true" />
          </span>
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="flex min-w-0 items-center gap-2">
              <span className="truncate text-sm font-medium text-fg">
                {existing.name}
              </span>
              <Badge tone="success" size="sm">
                {t("使用中")}
              </Badge>
            </span>
            <code className="block truncate font-mono text-xs text-faint">
              {existing.rootPath}
            </code>
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-4">
          <div className="flex flex-col items-center gap-3 rounded-xl border border-dashed border-line-strong/70 bg-inset/30 px-5 py-7 text-center">
            <span className="grid size-11 place-items-center rounded-2xl bg-brand-soft text-brand">
              <Folder size={20} aria-hidden="true" />
            </span>
            <div className="flex min-w-0 flex-col gap-0.5">
              <span className="text-sm font-medium text-fg">
                {root ? t("選択したフォルダ") : t("フォルダが選ばれていません")}
              </span>
              <code
                className="max-w-full truncate font-mono text-xs text-muted"
                title={root ?? undefined}
              >
                {root ?? t("未設定")}
              </code>
            </div>
            <Button
              size="sm"
              variant={root ? "default" : "primary"}
              onClick={() => void pickFolder()}
            >
              <Folder size={12} aria-hidden="true" />
              {t("フォルダを選択")}
            </Button>
            <span className="max-w-sm text-xs leading-5 text-faint">
              {t(
                "既存のフォルダを選んでください（新しいフォルダは作成されません）",
              )}
            </span>
          </div>

          <Field
            label={t("プロジェクト名")}
            hint={t("未入力の場合はフォルダ名が使われます")}
          >
            <Input
              placeholder={t("Project 名")}
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && root) create.mutate();
              }}
            />
          </Field>
        </div>
      )}

      {error && <ErrorBox error={error} />}

      <Row>
        <Button
          variant={existing ? "ghost" : "primary"}
          onClick={() => {
            if (existing) {
              setActiveProjectId(existing.id);
              onDone(existing.name);
              return;
            }
            create.mutate();
          }}
          disabled={!existing && !root}
        >
          {create.isPending ? <Spinner size={12} /> : <Folder size={12} />}
          {existing ? t("このプロジェクトを使う") : t("このフォルダを使う")}
        </Button>
        <Button variant="ghost" onClick={onSkip}>
          {t("あとから設定する")}
        </Button>
      </Row>
    </div>
  );
}
