import { useState } from "react";
import { Check, Folder, MessageSquare } from "lucide-react";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { useActiveProject, useModels, useProviders } from "@/lib/queries";
import { useUi } from "@/lib/ui";
import { useCreateThread } from "@/features/shell/useShellEvents";
import { Row } from "@/components/layout";
import { ErrorBadge } from "@/components/shared";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/cn";

/**
 * Step 3: confirm what was configured and open the first thread.
 *
 * Deliberately a summary rather than a fourth question. By this point the
 * wizard has asked for everything it needs, and the most useful last action is
 * the app's own screen with an empty transcript and its quick prompts — not
 * another form.
 */
export function StepStart({
  projectName,
  onStart,
}: {
  projectName: string | null;
  onStart: () => void;
}) {
  const providers = useProviders();
  const models = useModels();
  const activeProject = useActiveProject();
  const createThread = useCreateThread();
  const setSurface = useUi((s) => s.setSurface);

  const modelCount = models.data?.length ?? 0;
  const providerLabels = (providers.data ?? []).map((p) => p.label).join(", ");
  // Step 2 reports what it just created, but the user may also have skipped past
  // it with a project already selected from an earlier session — the summary
  // has to describe the real active project, not this run's return value.
  const folderName = projectName ?? activeProject?.name ?? null;
  const [error, setError] = useState<string | null>(null);

  const start = (surface: "chat" | "desktop-agent") => {
    setError(null);
    // On a fresh install the default surface is `chat`, whose threads carry no
    // project. Picking a folder in step 2 therefore had no effect on the first
    // thread at all: the file tree stayed empty and the folder only mattered
    // after the user discovered the agent surface. Switching here makes the
    // hand-off match the summary the user just confirmed. `useCreateThread`
    // reads the store at mutation time, so the surface must be set first.
    setSurface(surface);
    // Finish onboarding only once the thread exists. Marking it done first
    // dismissed the wizard even when thread creation failed, dropping the user
    // into the shell with no thread and no explanation.
    createThread.mutate(undefined, {
      onSuccess: () => onStart(),
      onError: (err) => setError(errorMessage(err) || t("エラー")),
    });
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <h2 className="text-xl font-semibold tracking-tight text-fg">
          {t("準備ができました")}
        </h2>
        <p className="text-sm text-muted">
          {t(
            "チャットを始められます。ファイルの読み書きやシェル実行は、実行のたびに確認が出ます。",
          )}
        </p>
      </div>

      <ul className="flex flex-col gap-2">
        <SummaryRow
          done={modelCount > 0}
          label={t("モデル")}
          value={
            modelCount > 0
              ? `${modelCount} ${t("件")} (${providerLabels})`
              : t("未設定")
          }
        />
        <SummaryRow
          done={folderName !== null}
          label={t("作業フォルダ")}
          value={
            folderName ?? t("未設定（チャットはプロジェクトなしで始まります）")
          }
        />
      </ul>

      <Row className="flex-wrap">
        <Button
          variant="primary"
          size="lg"
          onClick={() => start(folderName ? "desktop-agent" : "chat")}
          disabled={createThread.isPending}
        >
          {createThread.isPending ? (
            <Spinner size={13} />
          ) : (
            <MessageSquare size={13} />
          )}
          {folderName
            ? t("このフォルダでエージェントを始める")
            : t("チャットを始める")}
        </Button>
        {folderName && (
          <Button
            variant="ghost"
            size="lg"
            onClick={() => start("chat")}
            disabled={createThread.isPending}
          >
            {t("チャットを始める")}
          </Button>
        )}
        <span className="flex items-center gap-1 text-xs text-faint">
          <Folder size={10} aria-hidden="true" />
          {t("設定はここで変えられます")}
        </span>
      </Row>
      {error && <ErrorBadge error={error} />}
    </div>
  );
}

function SummaryRow({
  done,
  label,
  value,
}: {
  done: boolean;
  label: string;
  value: string;
}) {
  return (
    <li className="flex items-center gap-2.5 rounded-xl border border-line bg-surface px-3 py-3">
      <span
        aria-hidden="true"
        className={cn(
          "flex size-5 shrink-0 items-center justify-center rounded-full",
          done ? "bg-success-soft text-success" : "bg-inset text-faint",
        )}
      >
        <Check size={11} className={done ? undefined : "opacity-40"} />
      </span>
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="text-xs text-faint">{label}</span>
        <span className="truncate text-xs text-fg">{value}</span>
      </span>
    </li>
  );
}
