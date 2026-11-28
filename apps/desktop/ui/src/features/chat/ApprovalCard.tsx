/// Approval decision card for a run blocked on a destructive tool.
///
/// Accessibility: a mounted card used to call `.focus()` on the approve
/// button and `scrollIntoView`, stealing focus from wherever the user was
/// (often the composer). Focus is never taken automatically now — the card
/// announces itself through an `aria-live` region and stays in the tab order,
/// so keyboard users reach the decision with Tab instead of having it yanked
/// to them.

import { memo, useEffect, useState } from "react";
import { Clock3, ShieldCheck } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, useLanguage } from "@/lib/i18n";
import type { RunTimeline } from "@/lib/runEvents";
import { recordUxEvent } from "@/lib/uxEvents";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ErrorBadge } from "@/components/shared";
import { ApprovalDiffView } from "./ApprovalDiffView";
import { ApprovalCountdown, useApprovalRemaining } from "./ApprovalCountdown";
import { CommandRiskNotice } from "./CommandRiskNotice";

export const ApprovalCard = memo(function ApprovalCard({
  live,
}: {
  live: RunTimeline;
}) {
  // `memo` would skip this card on a locale change with an unchanged timeline.
  useLanguage();
  const approval = live.approval;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // `runEvents` replaces `next.approval` while a second request is pending in
  // the same run; the card stays mounted. Keep the acknowledgement paired with
  // the exact prompt it refers to and derive it during render: a passive
  // `useEffect` reset would leave a one-frame window where the old checkbox
  // state authorises the new (possibly more destructive) command. The key also
  // covers the arguments, which can change while the id is reused.
  const approvalId = approval?.approvalId;
  const approvalKey = `${approvalId ?? ""}:${JSON.stringify(approval?.arguments ?? null)}`;
  const [ack, setAck] = useState<{ key: string; value: boolean }>({
    key: "",
    value: false,
  });
  const acknowledged = ack.key === approvalKey && ack.value;
  const setAcknowledged = (value: boolean) =>
    setAck({ key: approvalKey, value });
  useEffect(() => {
    setBusy(false);
    setError(null);
  }, [approvalKey]);
  // Extending returns the new deadline before the event bus round-trip, so
  // the countdown restarts immediately. Keyed by the prompt so a stale value
  // can never make the next prompt look extended.
  const [extended, setExtended] = useState<{ key: string; value: string }>({
    key: "",
    value: "",
  });
  const expiresAt =
    extended.key === approvalKey ? extended.value : approval?.expiresAt;
  const remaining = useApprovalRemaining(expiresAt);
  const expired = remaining === 0;
  const dangerous = approval?.risk?.level === "dangerous";
  // Announce without moving focus: the run is blocked until this is answered,
  // so screen-reader users must learn about it, but taking focus would pull
  // sighted keyboard users out of the composer mid-typing.
  if (!approval) return null;
  const extend = async (seconds: number) => {
    const id = approval.approvalId;
    if (!id) return;
    setBusy(true);
    setError(null);
    try {
      const result = await api.extendApproval({ id, seconds });
      setExtended({ key: approvalKey, value: result.expires_at });
      recordUxEvent("approval_extended", { location: "card" });
    } catch (err) {
      setError(errorMessage(err) || t("エラー"));
    } finally {
      setBusy(false);
    }
  };
  const decide = async (
    grant: boolean,
    remember: boolean,
    rememberSession = false,
  ) => {
    const id = approval.approvalId;
    if (!id) return;
    // Never trust the disabled attribute alone: a stale acknowledgement from a
    // previous prompt must not authorise this one even if a click is forced.
    if (dangerous && grant && !acknowledged) return;
    setBusy(true);
    setError(null);
    try {
      await api.decideApproval({
        id,
        grant,
        remember,
        rememberSession,
        // The backend refuses a dangerous approval without this flag, so a
        // client that never rendered the warning cannot approve by omission.
        ...(dangerous && grant ? { acknowledgeRisk: true } : {}),
      });
      recordUxEvent("approval_decided", {
        location: "card",
        grant,
        remember,
        remember_session: rememberSession,
      });
    } catch (err) {
      // Without this a failed approval re-enabled the card silently while the
      // run stayed blocked waiting for an answer that would never arrive.
      setError(errorMessage(err) || t("エラー"));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div
      role="alertdialog"
      aria-modal="false"
      tabIndex={-1}
      aria-label={`${t("承認が必要です")}: ${approval.name}`}
      aria-describedby={`approval-args-${approval.approvalId ?? "pending"}`}
      className="rounded-lg border border-warning/25 bg-warning-soft outline-none"
    >
      <span role="status" className="sr-only">
        {`${t("承認が必要です")}: ${approval.name}`}
      </span>
      <div className="flex flex-wrap items-center gap-1.5 px-3 py-2 text-sm font-medium text-warning">
        <ShieldCheck size={15} className="shrink-0" aria-hidden="true" />
        {t("承認が必要です")} —{" "}
        <code className="rounded-sm bg-inset px-1 py-0.5 font-mono text-xs">
          {approval.name}
        </code>
        <ApprovalCountdown expiresAt={approval.expiresAt} className="ms-auto" />
      </div>
      {expired && (
        <p
          role="alert"
          className="mx-3 mb-2 rounded-md bg-danger-soft px-2 py-1 text-xs text-danger"
        >
          {t(
            "期限が切れたため、この操作は拒否されました。再実行すると新しい承認リクエストが出ます。",
          )}
        </p>
      )}
      <CommandRiskNotice
        risk={approval.risk}
        acknowledged={acknowledged}
        onAcknowledgedChange={dangerous ? setAcknowledged : undefined}
      />
      <div
        id={`approval-args-${approval.approvalId ?? "pending"}`}
        className="px-3 py-1"
      >
        <ApprovalDiffView
          toolName={approval.name}
          args={approval.arguments as Record<string, unknown>}
        />
      </div>
      <p className="mx-3 mb-1 text-2xs text-faint">
        {dangerous
          ? t(
              "危険なコマンドは「常に許可」に保存できません。実行のたびに確認します。",
            )
          : t(
              "「常に許可」は、このプロジェクト内でこのツールの同種の操作を記憶します。",
            )}
      </p>
      <div className="flex items-center justify-end gap-2 px-3 py-2">
        {error && (
          <span role="status" className="mr-auto">
            <ErrorBadge error={error} />
          </span>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              size="sm"
              disabled={busy || !approval.approvalId || expired}
              title={t("承認の期限を延長します")}
            >
              <Clock3 size={12} aria-hidden="true" />
              {t("+5分")}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={() => void extend(5 * 60)}>
              {t("5分延長")}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => void extend(15 * 60)}>
              {t("15分延長")}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => void extend(60 * 60)}>
              {t("1時間延長")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <Button
          size="sm"
          disabled={busy || !approval.approvalId || expired}
          onClick={() => void decide(false, false)}
        >
          {t("拒否")}
        </Button>
        {!dangerous && (
          <>
            <Button
              size="sm"
              disabled={busy || !approval.approvalId || expired}
              onClick={() => void decide(true, true)}
            >
              {t("常に許可")}
            </Button>
            <Button
              size="sm"
              disabled={busy || !approval.approvalId || expired}
              onClick={() => void decide(true, false, true)}
            >
              {t("このセッションで許可")}
            </Button>
          </>
        )}
        <Button
          variant="primary"
          size="sm"
          disabled={
            busy ||
            !approval.approvalId ||
            expired ||
            (dangerous && !acknowledged)
          }
          onClick={() => void decide(true, false)}
        >
          {t("今回のみ許可")}
        </Button>
      </div>
    </div>
  );
});
