import { t } from "@/app/i18n";
import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { CardSkeleton } from "@/components/loading";
import { ConfirmButton } from "@/components/modal";
import { ErrorBox, Title } from "@/components/shared";
import { useToast } from "@/components/toast";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { PushControl } from "./PushControl";

type Job = Row & {
  data: {
    name: string;
    target_type: "agent" | "conversation";
    target_id: string;
    prompt: string;
    cron: string;
    timezone: string;
    enabled: boolean;
    catch_up: boolean;
    next_at?: string | null;
  };
};

type JobRun = {
  id: string;
  status: string;
  scheduled_at: string;
  answer?: string;
  data?: { reason?: string };
};

export function Schedules() {
  const qc = useQueryClient();
  const { addToast } = useToast();
  const [error, setError] = useState<unknown>(null);
  const [selected, setSelected] = useState("");
  const [form, setForm] = useState({
    name: "",
    target_type: "agent",
    target_id: "",
    prompt: "",
    cron: "0 9 * * *",
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || "Asia/Tokyo",
    enabled: true,
    catch_up: false,
  });
  const jobs = useQuery<Job[]>({
    queryKey: ["/scheduled-jobs"],
    queryFn: () => api("/scheduled-jobs"),
  });
  const agents = useQuery<Row[]>({
    queryKey: ["/agents"],
    queryFn: () => api("/agents"),
  });
  const conversations = useQuery<Row[]>({
    queryKey: ["/conversations"],
    queryFn: () => api("/conversations?state=active"),
  });
  const notifications = useQuery<Row[]>({
    queryKey: ["/notifications"],
    queryFn: () => api("/notifications?unread=true"),
  });
  const runs = useQuery<JobRun[]>({
    queryKey: ["/scheduled-jobs", selected, "runs"],
    queryFn: () => api(`/scheduled-jobs/${selected}/runs`),
    enabled: !!selected,
  });
  const refresh = () => qc.invalidateQueries({ queryKey: ["/scheduled-jobs"] });
  const targets =
    form.target_type === "agent" ? agents.data || [] : conversations.data || [];
  const preset =
    (
      {
        "0 * * * *": "hourly",
        "0 9 * * *": "daily",
        "0 9 * * 1": "weekly",
      } as Record<string, string>
    )[form.cron] || "custom";

  function reportFailure(reason: unknown) {
    setError(reason);
    addToast({
      type: "error",
      title: t("定期実行の操作に失敗しました"),
      message: reason instanceof Error ? reason.message : String(reason),
    });
  }

  async function runNow(job: Job) {
    try {
      setError(null);
      await api(`/scheduled-jobs/${job.id}/run`, "POST");
      await refresh();
      addToast({ type: "success", title: t("定期実行を開始しました") });
    } catch (reason) {
      reportFailure(reason);
    }
  }

  async function toggleJob(job: Job) {
    const enabled = !job.data.enabled;
    try {
      setError(null);
      await api(`/scheduled-jobs/${job.id}`, "PATCH", { ...job.data, enabled });
      await refresh();
      addToast({
        type: "success",
        title: t(
          enabled ? "定期実行を有効にしました" : "定期実行を停止しました",
        ),
      });
    } catch (reason) {
      reportFailure(reason);
    }
  }

  async function deleteJob(job: Job) {
    try {
      setError(null);
      await api(`/scheduled-jobs/${job.id}`, "DELETE");
      await refresh();
      addToast({ type: "success", title: t("定期実行を削除しました") });
    } catch (reason) {
      reportFailure(reason);
    }
  }

  return (
    <section className="page">
      <Title
        title={t("定期実行")}
        sub={t(
          "Cron式またはフォーム相当の定期設定で、Agentと会話を自動実行します。",
        )}
      />
      <PushControl />
      <ErrorBox
        error={error || jobs.error || runs.error}
        onRetry={() => {
          setError(null);
          void jobs.refetch();
          if (selected) void runs.refetch();
        }}
      />
      <form
        className="stack"
        onSubmit={async (e) => {
          e.preventDefault();
          try {
            await api("/scheduled-jobs", "POST", form);
            setForm({ ...form, name: "", prompt: "", target_id: "" });
            void refresh();
          } catch (x) {
            setError(x);
          }
        }}
      >
        <input
          required
          placeholder={t("ジョブ名")}
          value={form.name}
          onChange={(e) => setForm({ ...form, name: e.target.value })}
        />
        <select
          value={form.target_type}
          onChange={(e) =>
            setForm({ ...form, target_type: e.target.value, target_id: "" })
          }
        >
          <option value="agent">{t("保存済みAgentを独立実行")}</option>
          <option value="conversation">{t("既存会話へ投稿")}</option>
        </select>
        <select
          required
          value={form.target_id}
          onChange={(e) => setForm({ ...form, target_id: e.target.value })}
        >
          <option value="">{t("対象を選択")}</option>
          {targets.map((t) => (
            <option key={t.id} value={t.id}>
              {t.data.name || t.data.title}
            </option>
          ))}
        </select>
        <textarea
          required
          placeholder={t("毎回送るプロンプト")}
          value={form.prompt}
          onChange={(e) => setForm({ ...form, prompt: e.target.value })}
        />
        <label>
          {t("頻度")}
          <select
            value={preset}
            onChange={(e) => {
              const cron = (
                {
                  hourly: "0 * * * *",
                  daily: "0 9 * * *",
                  weekly: "0 9 * * 1",
                } as Record<string, string>
              )[e.target.value];
              if (cron) setForm({ ...form, cron });
            }}
          >
            <option value="hourly">{t("毎時")}</option>
            <option value="daily">{t("毎日 09:00")}</option>
            <option value="weekly">{t("毎週月曜 09:00")}</option>
            <option value="custom">{t("Cron式を直接指定")}</option>
          </select>
        </label>
        <label>
          {t("Cron式（分 時 日 月 曜日）")}
          <input
            required
            value={form.cron}
            onChange={(e) => setForm({ ...form, cron: e.target.value })}
          />
        </label>
        <label>
          {t("タイムゾーン")}
          <input
            required
            value={form.timezone}
            onChange={(e) => setForm({ ...form, timezone: e.target.value })}
          />
        </label>
        <label>
          <input
            type="checkbox"
            checked={form.catch_up}
            onChange={(e) => setForm({ ...form, catch_up: e.target.checked })}
          />{" "}
          {t("停止中の最新1回を復旧時に実行")}
        </label>
        <Button type="submit">{t("定期実行を作成")}</Button>
      </form>
      {!!notifications.data?.length && (
        <section className="conversation-list">
          <h3>{t("未読通知")}</h3>
          {notifications.data.map((n) => (
            <article className="conversation-row" key={n.id}>
              <span>{n.data.title}</span>
              <Button
                variant="outline"
                onClick={async () => {
                  try {
                    await api(`/notifications/${n.id}`, "PATCH", {
                      read: true,
                    });
                    await qc.invalidateQueries({
                      queryKey: ["/notifications"],
                    });
                  } catch (reason) {
                    setError(reason);
                  }
                }}
              >
                {t("既読")}
              </Button>
            </article>
          ))}
        </section>
      )}
      <div className="conversation-list">
        {jobs.isLoading ? (
          <div aria-busy="true">
            <CardSkeleton />
            <CardSkeleton />
          </div>
        ) : jobs.isError ? null : jobs.data?.length ? (
          jobs.data.map((job) => (
            <article className="conversation-row" key={job.id}>
              <div>
                <b>{job.data.name}</b>
                <small>
                  {job.data.cron} · {job.data.timezone} {t("· 次回:")}{" "}
                  {job.data.next_at
                    ? new Date(job.data.next_at).toLocaleString()
                    : t("停止中")}
                </small>
              </div>
              <div className="row-actions">
                <Button variant="outline" onClick={() => setSelected(job.id)}>
                  {t("履歴")}
                </Button>
                <Button variant="outline" onClick={() => void runNow(job)}>
                  {t("今すぐ実行")}
                </Button>
                <Button variant="outline" onClick={() => void toggleJob(job)}>
                  {job.data.enabled ? t("停止") : t("有効化")}
                </Button>
                <ConfirmButton
                  variant="ghost"
                  className="icon danger"
                  title={t("定期実行を削除")}
                  message={t("定期実行を削除しますか？")}
                  confirmLabel={t("削除")}
                  onConfirm={() => void deleteJob(job)}
                >
                  ×
                </ConfirmButton>
              </div>
            </article>
          ))
        ) : error ? null : (
          <p className="empty">{t("定期実行はまだありません。")}</p>
        )}
      </div>
      {selected && (
        <section className="conversation-list">
          <h3>{t("実行履歴")}</h3>
          {runs.isLoading ? (
            <div aria-busy="true">
              <CardSkeleton />
            </div>
          ) : runs.isError ? null : runs.data?.length ? (
            runs.data.map((run) => (
              <article className="conversation-row" key={run.id}>
                <div>
                  <b>{run.status}</b>
                  <small>
                    {new Date(run.scheduled_at).toLocaleString()}{" "}
                    {run.data?.reason ? `· ${run.data.reason}` : ""}
                  </small>
                  {run.answer && <p>{run.answer}</p>}
                </div>
              </article>
            ))
          ) : error ? null : (
            <p className="empty">{t("実行履歴はまだありません。")}</p>
          )}
        </section>
      )}
    </section>
  );
}
