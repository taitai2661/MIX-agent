import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Clock3, Pause, Play, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { Card, Page, Row } from "@/components/layout";
import { EmptyState, ErrorBadge } from "@/components/shared";
import { Alert } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Select, Textarea } from "@/components/ui/input";
import type { ScheduleJobView } from "@/lib/types";

const PRESETS: Array<{ label: string; cron: string }> = [
  { label: "毎時 (0 * * * *)", cron: "0 * * * *" },
  { label: "毎日 9:00", cron: "0 9 * * *" },
  { label: "平日 9:00 (0 9 * * mon-fri)", cron: "0 9 * * mon-fri" },
  { label: "毎週月曜 8:00", cron: "0 8 * * mon" },
  { label: "毎15分 (*/15 * * * *)", cron: "*/15 * * * *" },
];

export function SchedulesPane() {
  const queryClient = useQueryClient();
  const jobs = useQuery({ queryKey: ["schedule_jobs"], queryFn: api.listScheduleJobs });
  const models = useQuery({ queryKey: ["models"], queryFn: api.listModels });
  const [draftName, setDraftName] = useState("");
  const [draftCron, setDraftCron] = useState("0 9 * * *");
  const [draftPrompt, setDraftPrompt] = useState("");
  const [draftModel, setDraftModel] = useState("");
  const [draftMode, setDraftMode] = useState("chat");
  const [preview, setPreview] = useState<string[] | null>(null);
  const [cronError, setCronError] = useState<string | null>(null);

  // Live preview of the expression as the user types.
  useEffect(() => {
    const handle = setTimeout(() => {
      if (!draftCron.trim()) { setPreview(null); setCronError(null); return; }
      api.previewCron(draftCron).then(
        (times) => { setPreview(times); setCronError(null); },
        (e) => { setPreview(null); setCronError(errorMessage(e)); },
      );
    }, 300);
    return () => clearTimeout(handle);
  }, [draftCron]);

  const create = useMutation({
    mutationFn: () => api.createScheduleJob({
      name: draftName,
      cron: draftCron,
      prompt: draftPrompt,
      conversation_id: null,
      model_id: draftModel || null,
      mode: draftMode,
    }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["schedule_jobs"] });
      setDraftName(""); setDraftPrompt("");
    },
  });

  const toggle = useMutation({
    mutationFn: (args: { id: string; enabled: boolean }) =>
      api.setScheduleJobEnabled(args.id, args.enabled),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["schedule_jobs"] }),
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteScheduleJob(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["schedule_jobs"] }),
  });

  const disabled = !draftName || !draftPrompt.trim() || !!cronError;

  return (
    <Page
      title={t("スケジュール")}
      sub="Cron で定期実行。アプリ起動中 (および次回起動時に 1 回) 自動実行されます。"
    >
      <Card title="定期ジョブを追加" icon={<Clock3 size={13} />}>
        <Row className="gap-2 [&>*]:flex-1">
          <Input placeholder="Job 名" value={draftName} onChange={(e) => setDraftName(e.target.value)} />
          <Input
            className="font-mono"
            placeholder="Cron 式 (例: 0 9 * * *)"
            value={draftCron}
            onChange={(e) => setDraftCron(e.target.value)}
          />
        </Row>

        <Row className="gap-1.5">
          {PRESETS.map((p) => (
            <Button
              key={p.cron}
              size="sm"
              aria-pressed={draftCron === p.cron}
              onClick={() => setDraftCron(p.cron)}
              className={cn(draftCron === p.cron && "border-brand bg-brand-soft text-brand")}
            >
              {p.label}
            </Button>
          ))}
        </Row>

        {cronError && <Alert tone="danger">{cronError}</Alert>}
        {preview && (
          <p className="m-0 text-2xs text-muted">
            Next: {preview.slice(0, 3).map((ts) => new Date(ts).toLocaleString()).join(" · ")}
          </p>
        )}

        <Textarea
          placeholder="実行プロンプト…"
          value={draftPrompt}
          onChange={(e) => setDraftPrompt(e.target.value)}
        />
        <Row className="gap-2 [&>*]:flex-1">
          <Select value={draftMode} onChange={(e) => setDraftMode(e.target.value)}>
            <option value="chat">chat</option>
            <option value="thinking">thinking</option>
            <option value="agent">agent</option>
          </Select>
          <Select
            className="flex-[2]"
            value={draftModel}
            onChange={(e) => setDraftModel(e.target.value)}
          >
            <option value="">モデル: 会話の既定</option>
            {(models.data ?? []).map((m) => (
              <option key={m.id} value={m.id}>{m.label ?? m.id}</option>
            ))}
          </Select>
        </Row>
        <Row>
          <Button
            variant="primary"
            disabled={disabled || create.isPending}
            onClick={() => create.mutate()}
          >
            <Clock3 size={12} /> 登録
          </Button>
          {create.isError && <ErrorBadge error={create.error} />}
        </Row>
      </Card>

      <h2 className="text-sm font-semibold text-fg">登録済みジョブ</h2>
      {(jobs.data ?? []).length === 0 ? (
        <EmptyState icon={<Clock3 size={22} />} title="ジョブはまだありません。" />
      ) : (
        <div className="overflow-hidden rounded-lg border border-line">
          <table className="w-full border-collapse text-xs">
            <thead>
              <tr className="border-b border-line bg-inset text-2xs text-muted">
                <th className="px-3 py-1.5 text-start font-medium">名前</th>
                <th className="px-3 py-1.5 text-start font-medium">Cron</th>
                <th className="px-3 py-1.5 text-start font-medium">状態</th>
                <th className="px-3 py-1.5 text-start font-medium">次回</th>
                <th className="px-3 py-1.5 text-start font-medium">最終実行</th>
                <th className="w-20 px-3 py-1.5" />
              </tr>
            </thead>
            <tbody className="divide-y divide-line">
              {(jobs.data ?? []).map((j: ScheduleJobView) => (
                <tr key={j.id} className="transition-colors hover:bg-hover">
                  <td className="px-3 py-1.5 font-medium text-fg">{j.name}</td>
                  <td className="px-3 py-1.5 font-mono text-muted">{j.cron}</td>
                  <td className="px-3 py-1.5">
                    {j.enabled
                      ? <Badge tone="success" size="sm">有効</Badge>
                      : <Badge size="sm">停止</Badge>}
                  </td>
                  <td className="px-3 py-1.5 text-muted">
                    {j.next_run_at ? new Date(j.next_run_at).toLocaleString() : "—"}
                  </td>
                  <td className="px-3 py-1.5 text-muted">
                    {j.last_run_at ? new Date(j.last_run_at).toLocaleString() : "—"}
                  </td>
                  <td className="px-3 py-1.5">
                    <Row align="end" className="flex-nowrap gap-0.5">
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        title={j.enabled ? "Stop" : "Enable"}
                        aria-label={j.enabled ? "Stop" : "Enable"}
                        onClick={() => toggle.mutate({ id: j.id, enabled: !j.enabled })}
                      >
                        {j.enabled ? <Pause size={13} /> : <Play size={13} />}
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        className="text-danger hover:text-danger"
                        title={t("削除")}
                        aria-label={t("削除")}
                        onClick={() => remove.mutate(j.id)}
                      >
                        <Trash2 size={13} />
                      </Button>
                    </Row>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Page>
  );
}
