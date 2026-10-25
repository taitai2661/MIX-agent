import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Folder } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { PROJECTS_QUERY_KEY, useProjects } from "@/lib/queries";
import { useUi } from "@/lib/ui";
import { Field, Row } from "@/components/layout";
import { ErrorBox } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
    const path = await openDialog({ directory: true, multiple: false });
    if (typeof path === "string" && path) {
      setRoot(path);
      setError(null);
    }
  };

  const existing = projects.data?.[0] ?? null;

  return (
    <div className="flex flex-col gap-3">
      <p className="m-0 text-xs text-muted">
        {t(
          "MIX-agent がファイルを読む・書く場所です。プロジェクトを作ると、そのフォルダの中だけが対象になります。",
        )}
      </p>

      {existing ? (
        <Row>
          <Folder size={12} className="text-muted" />
          <span className="text-xs text-fg">{existing.name}</span>
          <code className="min-w-0 truncate font-mono text-xs text-faint">
            {existing.rootPath}
          </code>
        </Row>
      ) : (
        <>
          {/* Not wrapped in `Field`: its `<label>` would swallow the picker
              button's accessible name. */}
          <div className="flex flex-col gap-1.5">
            <Label>{t("フォルダ")}</Label>
            <Row>
              <code className="min-w-0 flex-1 truncate font-mono text-xs text-muted">
                {root ?? t("未設定")}
              </code>
              <Button size="sm" onClick={() => void pickFolder()}>
                <Folder size={12} /> {t("フォルダを選択")}
              </Button>
            </Row>
            <span className="text-xs text-faint">
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
        </>
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
