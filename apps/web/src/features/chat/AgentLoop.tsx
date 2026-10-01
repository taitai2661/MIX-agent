import { t } from "@/app/i18n";
import { api } from "@/app/api";
import { Button } from "@/components/button";
import { useState } from "react";

import type { Run, RunCheckpoint } from "./types";

export function CheckpointsPanel({
  run,
  onError,
  onResumed,
}: {
  run: Run;
  onError: (err: unknown) => void;
  onResumed: () => void;
}) {
  const checkpoints: RunCheckpoint[] = run.checkpoints || [];
  const [selected, setSelected] = useState<string>("");
  const [busy, setBusy] = useState(false);

  if (checkpoints.length === 0) {
    return (
      <div className="run-resume">
        <Button
          variant="outline"
          disabled={busy}
          onClick={async () => {
            if (!confirm(t("結果不明の操作は再実行しません。外部状態を確認してから続けてください。"))) {
              return;
            }
            setBusy(true);
            try {
              await api("/runs/" + run.id + "/resume", "POST", {
                acknowledge_unknown_result: true,
              });
              onResumed();
            } catch (e) {
              onError(e);
            } finally {
              setBusy(false);
            }
          }}
        >
          {t("確認して再開")}
        </Button>
      </div>
    );
  }

  return (
    <div className="run-resume" role="group" aria-label={t("再開ポイント")}>
      <p className="run-resume-title">{t("途中再開ポイントから再開")}</p>
      <ul className="checkpoint-list">
        {checkpoints.slice(0, 8).map((ck) => (
          <li key={ck.id}>
            <label className="checkpoint-row">
              <input
                type="radio"
                name="checkpoint"
                value={ck.id}
                checked={selected === ck.id}
                onChange={() => setSelected(ck.id)}
              />
              <span>
                <b>{t("ステップ")} {ck.step}</b>
                <small>
                  {ck.tool_count} {t("Tool Call")} · {ck.trigger}
                  {ck.created_at ? ` · ${new Date(ck.created_at).toLocaleString()}` : ""}
                </small>
              </span>
            </label>
          </li>
        ))}
      </ul>
      <div className="run-resume-actions">
        <Button
          variant="outline"
          disabled={busy || !selected}
          onClick={async () => {
            if (!confirm(t("選択したチェックポイントから再開します。結果不明の操作は破棄されます。"))) {
              return;
            }
            setBusy(true);
            try {
              await api("/runs/" + run.id + "/resume", "POST", {
                acknowledge_unknown_result: true,
                from_checkpoint: selected,
              });
              onResumed();
            } catch (e) {
              onError(e);
            } finally {
              setBusy(false);
            }
          }}
        >
          {t("チェックポイントから再開")}
        </Button>
        <Button
          variant="ghost"
          disabled={busy}
          onClick={async () => {
            if (!confirm(t("結果不明の操作は再実行しません。外部状態を確認してから続けてください。"))) {
              return;
            }
            setBusy(true);
            try {
              await api("/runs/" + run.id + "/resume", "POST", {
                acknowledge_unknown_result: true,
              });
              onResumed();
            } catch (e) {
              onError(e);
            } finally {
              setBusy(false);
            }
          }}
        >
          {t("最初から再開")}
        </Button>
      </div>
    </div>
  );
}

export function BudgetExtensionPanel({
  runId,
  request,
  used,
  max,
  onError,
  onResolved,
}: {
  runId: string;
  request: NonNullable<Run["budget_extension_request"]>;
  used: number;
  max: number;
  onError: (err: unknown) => void;
  onResolved: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const decide = async (grant: boolean) => {
    setBusy(true);
    try {
      await api("/runs/" + runId + "/budget-extension", "POST", { grant });
      onResolved();
    } catch (e) {
      onError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="run-resume budget-extension" role="group" aria-label={t("予算延長")}>
      <p className="run-resume-title">{t("実行予算の上限に近づいています")}</p>
      <ul className="checkpoint-list">
        <li>
          <span>{t("Tool Call")}: {request.tool_calls_used} / {request.tool_calls_limit}</span>
        </li>
        <li>
          <span>{t("ステップ")}: {request.steps_used} / {request.steps_limit}</span>
        </li>
        <li>
          <span>{t("経過時間")}: {request.elapsed_seconds} / {request.max_seconds}秒</span>
        </li>
        <li>
          <span>{t("延長回数")}: {used} / {max}</span>
        </li>
      </ul>
      <p className="run-resume-detail">
        {t("このまま続けると停止します。予算を延長しますか?")}
      </p>
      <div className="run-resume-actions">
        <Button disabled={busy || used > max} onClick={() => decide(true)}>
          {t("延長する")}
        </Button>
        <Button variant="ghost" disabled={busy} onClick={() => decide(false)}>
          {t("このまま停止")}
        </Button>
      </div>
    </div>
  );
}

export function StagnationPanel({ findings }: { findings: { code: string; severity?: string; tool_id?: string; count?: number }[] }) {
  if (!findings.length) return null;
  return (
    <div className="run-stagnation" role="status">
      <p className="run-stagnation-title">{t("停滞シグナル")}</p>
      <ul>
        {findings.map((finding, index) => (
          <li key={`${finding.code}-${index}`}>
            <code>{finding.code}</code>
            {finding.tool_id ? <span> · {finding.tool_id}</span> : null}
            {typeof finding.count === "number" ? <span> · {finding.count}回</span> : null}
          </li>
        ))}
      </ul>
    </div>
  );
}
