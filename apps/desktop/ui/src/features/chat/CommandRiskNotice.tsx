import { AlertTriangle, ShieldAlert } from "lucide-react";
import { t } from "@/lib/i18n";
import type { RiskAssessment } from "@/lib/runEvents";

/** Human label for a classifier code.
 *
 *  Codes are stable and shared with `tools/src/risk.rs`; translating here means
 *  the Rust side never has to carry UI copy, and an unknown code still renders
 *  its concrete matched text instead of disappearing.
 */
const REASON_LABELS: Record<string, string> = {
  recursive_delete: "再帰的な削除",
  disk_write: "ディスク・デバイスへの書き込み",
  outside_write: "ワークスペース外への書き込み",
  permissions_world_writable: "権限の一括変更",
  pipe_to_shell: "取得したスクリプトの直接実行",
  dynamic_eval: "動的コードの実行",
  privilege_escalation: "管理者権限の使用",
  fork_bomb: "無限ループ・プロセス爆発",
  git_destructive: "Git 履歴の破壊的操作",
  credential_access: "認証情報へのアクセス",
  network_exfiltration: "外部へのデータ送信",
  system_power: "システムの停止・再起動",
  container_destructive: "コンテナ・インフラの破壊的操作",
  database_destructive: "データベースの破壊的操作",
  package_install: "依存パッケージのインストール",
  network_access: "ネットワークアクセス",
  git_write: "Git への書き込み",
  container: "コンテナ操作",
  permission_change: "権限の変更",
  process_control: "プロセス操作",
  redirect_outside: "ワークスペース外への出力",
  background_service: "常駐サービスの操作",
};

export function reasonLabel(code: string): string {
  return t(REASON_LABELS[code] ?? code);
}

/**
 * Why a command is not an ordinary `terminal` call.
 *
 * For `dangerous` commands the caller also passes the acknowledgement state:
 * the approve buttons stay disabled until the box is ticked, and the backend
 * independently rejects an approval that does not set `acknowledge_risk`.
 */
export function CommandRiskNotice({
  risk,
  acknowledged,
  onAcknowledgedChange,
}: {
  risk?: RiskAssessment | null;
  acknowledged?: boolean;
  onAcknowledgedChange?: (value: boolean) => void;
}) {
  if (!risk || risk.level === "safe") return null;
  const dangerous = risk.level === "dangerous";

  return (
    <div
      role={dangerous ? "alert" : "note"}
      className={
        dangerous
          ? "mx-3 my-2 rounded-md border border-danger/40 bg-danger-soft px-2.5 py-2 text-xs text-danger"
          : "mx-3 my-2 rounded-md border border-warning/30 bg-warning-soft px-2.5 py-2 text-xs text-warning"
      }
    >
      <p className="m-0 flex items-center gap-1.5 font-semibold">
        {dangerous ? (
          <ShieldAlert size={13} aria-hidden="true" />
        ) : (
          <AlertTriangle size={13} aria-hidden="true" />
        )}
        {dangerous ? t("危険なコマンドです") : t("確認を推奨するコマンドです")}
      </p>
      <ul className="mt-1 mb-0 list-disc space-y-0.5 ps-4">
        {risk.reasons.map((reason, index) => (
          <li key={`${reason.code}-${index}`}>
            {reasonLabel(reason.code)}
            {reason.detail && (
              <code className="ms-1 break-all rounded-sm bg-inset px-1 py-0.5 font-mono text-2xs text-muted">
                {reason.detail}
              </code>
            )}
          </li>
        ))}
      </ul>
      {dangerous && !onAcknowledgedChange && (
        <p className="mt-1.5 mb-0">
          {t(
            "この操作は取り消せない可能性があります。内容を確認してから許可してください。",
          )}
        </p>
      )}
      {dangerous && onAcknowledgedChange && (
        <label className="mt-2 flex cursor-pointer items-start gap-1.5 font-medium">
          <input
            type="checkbox"
            className="mt-0.5 size-3.5 shrink-0 accent-danger"
            checked={Boolean(acknowledged)}
            onChange={(event) => onAcknowledgedChange(event.target.checked)}
          />
          <span>{t("内容を理解しました。このコマンドを実行します。")}</span>
        </label>
      )}
    </div>
  );
}
