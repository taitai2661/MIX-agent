import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Clock3, Pause, Play } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import type { Mode } from "@/lib/ui";
import { Card, DataTable, Page, Row, td, tdStrong } from "@/components/layout";
import {
  EmptyState,
  ErrorBadge,
  ErrorBox,
  TableSkeleton,
} from "@/components/shared";
import { DeleteIconButton, SectionHeading } from "@/components/common";
import { Button } from "@/components/ui/button";
import { YesNo } from "@/components/ui/status";
import { Input, Select, Textarea } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { useModels } from "@/lib/queries";
import type { ScheduleJobView } from "@/lib/types";

/** Labels are translated at render, not at module scope: a module-level `t()`
 *  captures whatever language happened to be active when the bundle was first
 *  evaluated and never updates. */
const PRESETS: Array<{ label: string; cron: string }> = [
  { label: "毎時 (0 * * * *)", cron: "0 * * * *" },
  { label: "毎日 9:00", cron: "0 9 * * *" },
  { label: "平日 9:00 (0 9 * * mon-fri)", cron: "0 9 * * mon-fri" },
  { label: "毎週月曜 8:00", cron: "0 8 * * mon" },
  { label: "毎15分 (*/15 * * * *)", cron: "*/15 * * * *" },
];

export function SchedulesPane() {
  const queryClient = useQueryClient();
  const jobs = useQuery({
    queryKey: ["schedule_jobs"],
    queryFn: api.listScheduleJobs,
  });
  const models = useModels();
  const [draftName, setDraftName] = useState("");
  const [draftCron, setDraftCron] = useState("0 9 * * *");
  const [draftPrompt, setDraftPrompt] = useState("");
  const [draftModel, setDraftModel] = useState("");
  const [draftMode, setDraftMode] = useState<Mode>("chat");
  const [preview, setPreview] = useState<string[] | null>(null);
  const [cronError, setCronError] = useState<string | null>(null);

  // Live preview of the expression as the user types. The AbortController
  // drops a slower earlier response so it cannot overwrite the newest preview.
  useEffect(() => {
    const controller = new AbortController();
    const handle = setTimeout(() => {
      if (!draftCron.trim()) {
        setPreview(null);
        setCronError(null);
        return;
      }
      api.previewCron(draftCron).then(
        (times) => {
          if (!controller.signal.aborted) {
            setPreview(times);
            setCronError(null);
          }
        },
        (e) => {
          if (!controller.signal.aborted) {
            setPreview(null);
            setCronError(errorMessage(e));
          }
        },
      );
    }, 300);
    return () => {
      controller.abort();
      clearTimeout(handle);
    };
  }, [draftCron]);

  const create = useMutation({
    mutationFn: () =>
      api.createScheduleJob({
        name: draftName,
        cron: draftCron,
        prompt: draftPrompt,
        conversation_id: null,
        model_id: draftModel || null,
        mode: draftMode,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["schedule_jobs"] });
      setDraftName("");
      setDraftPrompt("");
    },
  });

  const toggle = useMutation({
    mutationFn: (args: { id: string; enabled: boolean }) =>
      api.setScheduleJobEnabled(args.id, args.enabled),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["schedule_jobs"] }),
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteScheduleJob(id),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["schedule_jobs"] }),
  });

  const disabled =
    !draftName.trim() ||
    !draftCron.trim() ||
    !draftPrompt.trim() ||
    !!cronError;

  return (
    <Page
      title={t("スケジュール")}
      sub={t(
        "Cron で定期実行。アプリ起動中 (および次回起動時に 1 回) 自動実行されます。",
      )}
    >
      <Card title={t("定期ジョブを追加")} icon={<Clock3 size={13} />}>
        <div className="grid gap-2 sm:grid-cols-2">
          <Input
            placeholder={t("Job 名")}
            aria-label={t("Job 名")}
            value={draftName}
            onChange={(e) => setDraftName(e.target.value)}
          />
          <Input
            className="font-mono"
            placeholder={t("Cron 式 (例: 0 9 * * *)")}
            aria-label={t("Cron 式")}
            value={draftCron}
            onChange={(e) => setDraftCron(e.target.value)}
          />
        </div>

        <Row className="gap-1.5">
          {PRESETS.map((p) => (
            <Button
              key={p.cron}
              size="sm"
              selected={draftCron === p.cron}
              onClick={() => setDraftCron(p.cron)}
            >
              {t(p.label)}
            </Button>
          ))}
        </Row>

        {cronError && <ErrorBox error={cronError} />}
        {preview && (
          <p className="m-0 text-xs text-muted">
            {t("次回")}:{" "}
            {preview
              .slice(0, 3)
              .map((ts) => {
                const date = new Date(ts);
                return Number.isNaN(date.getTime())
                  ? "—"
                  : date.toLocaleString();
              })
              .join(" · ")}
          </p>
        )}

        <Textarea
          placeholder={t("実行プロンプト…")}
          value={draftPrompt}
          onChange={(e) => setDraftPrompt(e.target.value)}
        />
        <div className="grid gap-2 sm:grid-cols-2">
          <Select
            aria-label={t("モード")}
            value={draftMode}
            onChange={(e) => setDraftMode(e.target.value as Mode)}
          >
            <option value="chat">{t("チャット")}</option>
            <option value="thinking">{t("思考")}</option>
            <option value="agent">{t("エージェント")}</option>
          </Select>
          <Select
            aria-label={t("モデル")}
            value={draftModel}
            onChange={(e) => setDraftModel(e.target.value)}
          >
            <option value="">{t("モデル: 会話の既定")}</option>
            {(models.data ?? []).map((m) => (
              <option key={m.id} value={m.id}>
                {m.label ?? m.id}
              </option>
            ))}
          </Select>
        </div>
        <Row>
          <Button
            variant="primary"
            disabled={disabled || create.isPending}
            onClick={() => create.mutate()}
          >
            {create.isPending ? <Spinner size={12} /> : <Clock3 size={12} />}{" "}
            {t("登録")}
          </Button>
          {create.isError && <ErrorBadge error={create.error} />}
        </Row>
      </Card>

      <SectionHeading>{t("登録済みジョブ")}</SectionHeading>
      {jobs.isPending ? (
        <TableSkeleton />
      ) : jobs.isError ? (
        <ErrorBox error={jobs.error} />
      ) : (jobs.data ?? []).length === 0 ? (
        <EmptyState
          icon={<Clock3 size={22} />}
          title={t("ジョブはまだありません。")}
        />
      ) : (
        <DataTable
          columns={[t("名前"), "Cron", t("状態"), t("次回"), t("最終実行"), ""]}
          rows={jobs.data ?? []}
          rowKey={(j: ScheduleJobView) => j.id}
        >
          {(j) => (
            <>
              <td className={tdStrong}>{j.name}</td>
              <td className={`${td} font-mono`}>{j.cron}</td>
              <td className={td}>
                <YesNo
                  value={j.enabled}
                  onLabel={t("有効")}
                  offLabel={t("停止中")}
                  trueTone="success"
                />
              </td>
              <td className={td}>
                {(() => {
                  if (!j.next_run_at) return "—";
                  const d = new Date(j.next_run_at);
                  return Number.isNaN(d.getTime()) ? "—" : d.toLocaleString();
                })()}
              </td>
              <td className={td}>
                {(() => {
                  if (!j.last_run_at) return "—";
                  const d = new Date(j.last_run_at);
                  return Number.isNaN(d.getTime()) ? "—" : d.toLocaleString();
                })()}
              </td>
              <td className={`${td} w-20`}>
                <Row align="end" className="flex-nowrap gap-0.5">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    title={j.enabled ? t("停止") : t("有効にする")}
                    aria-label={
                      j.enabled
                        ? tf("{name}を停止", { name: j.name })
                        : tf("{name}を有効にする", { name: j.name })
                    }
                    onClick={() =>
                      toggle.mutate({ id: j.id, enabled: !j.enabled })
                    }
                  >
                    {j.enabled ? <Pause size={13} /> : <Play size={13} />}
                  </Button>
                  <DeleteIconButton
                    title={t("ジョブを削除しますか？")}
                    description={j.name}
                    pending={remove.isPending}
                    onDelete={() => remove.mutate(j.id)}
                  />
                </Row>
              </td>
            </>
          )}
        </DataTable>
      )}
    </Page>
  );
}
