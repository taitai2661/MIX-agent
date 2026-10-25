import { memo, useState } from "react";
import { ChevronDown, ShieldAlert } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import type { ApprovalRequest } from "@/lib/runEvents";
import { Button } from "@/components/ui/button";

export const ApprovalBanner = memo(function ApprovalBanner({
  approval,
}: {
  approval: ApprovalRequest;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const scrollToCard = () => {
    const card = document.querySelector('[role="alertdialog"]');
    if (card) {
      card.scrollIntoView({ behavior: "smooth", block: "center" });
      const firstBtn = card.querySelector("button");
      firstBtn?.focus();
    }
  };

  const decide = async (grant: boolean) => {
    const id = approval.approvalId;
    if (!id) return;
    setBusy(true);
    setError(null);
    try {
      await api.decideApproval({ id, grant, remember: false });
    } catch (err) {
      setError(errorMessage(err) || t("エラー"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      role="alert"
      className="sticky top-0 z-20 flex shrink-0 items-center justify-between gap-2 border-b border-warning/30 bg-warning/10 px-4 py-2 text-xs text-warning shadow-xs backdrop-blur-md"
    >
      <div className="flex min-w-0 items-center gap-2">
        <ShieldAlert size={15} className="shrink-0 text-warning animate-mix-pulse" />
        <span className="font-semibold">{t("承認待ち")}:</span>
        <code className="rounded bg-surface/70 px-1.5 py-0.5 font-mono text-2xs text-fg">
          {approval.name}
        </code>
        {error && <span className="text-danger text-2xs">({error})</span>}
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        <Button
          variant="ghost"
          size="xs"
          onClick={scrollToCard}
          className="h-6 gap-1 px-2 text-2xs text-warning hover:bg-warning/20 hover:text-warning"
          title={t("詳細を確認")}
        >
          <span>{t("詳細を確認")}</span>
          <ChevronDown size={12} />
        </Button>
        <Button
          variant="outline"
          size="xs"
          disabled={busy || !approval.approvalId}
          onClick={() => void decide(false)}
          className="h-6 px-2 text-2xs border-warning/30 text-warning hover:bg-warning/20"
        >
          {t("拒否")}
        </Button>
        <Button
          variant="primary"
          size="xs"
          disabled={busy || !approval.approvalId}
          onClick={() => void decide(true)}
          className="h-6 px-2 text-2xs"
        >
          {t("今回のみ許可")}
        </Button>
      </div>
    </div>
  );
});
