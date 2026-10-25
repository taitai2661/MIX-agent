/// Approval decision card for a run blocked on a destructive tool.
///
/// Accessibility: a mounted card used to call `.focus()` on the approve
/// button and `scrollIntoView`, stealing focus from wherever the user was
/// (often the composer). Focus is never taken automatically now — the card
/// announces itself through an `aria-live` region and stays in the tab order,
/// so keyboard users reach the decision with Tab instead of having it yanked
/// to them.

import { memo, useState } from "react";
import { ShieldCheck } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import type { RunTimeline } from "@/lib/runEvents";
import { Button } from "@/components/ui/button";
import { ErrorBadge } from "@/components/shared";
import { ApprovalDiffView } from "./ApprovalDiffView";

export const ApprovalCard = memo(function ApprovalCard({
  live,
}: {
  live: RunTimeline;
}) {
  const approval = live.approval;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Announce without moving focus: the run is blocked until this is answered,
  // so screen-reader users must learn about it, but taking focus would pull
  // sighted keyboard users out of the composer mid-typing.
  if (!approval) return null;
  const decide = async (grant: boolean, remember: boolean) => {
    const id = approval.approvalId;
    if (!id) return;
    setBusy(true);
    setError(null);
    try {
      await api.decideApproval({ id, grant, remember });
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
      aria-label={`${t("承認が必要です")}: ${approval.name}`}
      aria-describedby={`approval-args-${approval.approvalId ?? "pending"}`}
      className="rounded-lg border border-warning/25 bg-warning-soft"
    >
      <span role="status" className="sr-only">
        {`${t("承認が必要です")}: ${approval.name}`}
      </span>
      <div className="flex items-center gap-1.5 px-3 py-2 text-sm font-medium text-warning">
        <ShieldCheck size={15} className="shrink-0" aria-hidden="true" />
        {t("承認が必要です")} —{" "}
        <code className="rounded-sm bg-inset px-1 py-0.5 font-mono text-xs">
          {approval.name}
        </code>
      </div>
      <div className="px-3 py-1">
        <ApprovalDiffView
          toolName={approval.name}
          args={approval.arguments as Record<string, unknown>}
        />
      </div>
      <div className="flex items-center justify-end gap-2 px-3 py-2">
        {error && (
          <span role="status" className="mr-auto">
            <ErrorBadge error={error} />
          </span>
        )}
        <Button
          size="sm"
          disabled={busy || !approval.approvalId}
          onClick={() => void decide(false, false)}
        >
          {t("拒否")}
        </Button>
        <Button
          size="sm"
          disabled={busy || !approval.approvalId}
          onClick={() => void decide(true, true)}
        >
          {t("常に許可")}
        </Button>
        <Button
          variant="primary"
          size="sm"
          disabled={busy || !approval.approvalId}
          onClick={() => void decide(true, false)}
        >
          {t("今回のみ許可")}
        </Button>
      </div>
    </div>
  );
});
