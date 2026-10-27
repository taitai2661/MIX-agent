import { memo, useCallback, useEffect, useState } from "react";
import {
  ChevronDown,
  ShieldAlert,
  Terminal,
  FileCode,
  Wrench,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { chord } from "@/lib/platform";
import type { PendingApproval } from "@/lib/runEvents";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/layout";

export const ApprovalBanner = memo(function ApprovalBanner({
  approval,
}: {
  approval: PendingApproval;
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

  const decide = useCallback(
    async (grant: boolean) => {
      const id = approval.approvalId;
      if (!id || busy) return;
      setBusy(true);
      setError(null);
      try {
        await api.decideApproval({ id, grant, remember: false });
      } catch (err) {
        setError(errorMessage(err) || t("エラー"));
      } finally {
        setBusy(false);
      }
    },
    [approval.approvalId, busy],
  );

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      // ⌘+Enter or Ctrl+Enter approves the pending request
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        void decide(true);
        return;
      }
      // Escape rejects only when no open modal dialog / menu is active
      if (e.key === "Escape") {
        if (
          document.querySelector(
            "[role='dialog'][data-modal='true'], [role='menu']",
          )
        ) {
          return;
        }
        e.preventDefault();
        e.stopPropagation();
        void decide(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [decide]);

  const isTerminal = approval.name.toLowerCase().includes("terminal");
  const isFileEdit =
    approval.name.toLowerCase().includes("edit") ||
    approval.name.toLowerCase().includes("file");

  return (
    <div
      role="alert"
      className="sticky top-0 z-20 flex shrink-0 items-center justify-between gap-2 border-b border-warning/30 bg-warning/10 px-4 py-2 text-xs text-warning shadow-xs backdrop-blur-md"
    >
      <div className="flex min-w-0 items-center gap-2">
        <ShieldAlert
          size={15}
          className="shrink-0 text-warning animate-mix-pulse"
        />
        <span className="font-semibold">{t("承認待ち")}:</span>
        <span className="inline-flex items-center gap-1 rounded bg-surface/80 px-1.5 py-0.5 font-mono text-2xs text-fg shadow-2xs">
          {isTerminal ? (
            <Terminal size={11} className="text-warning shrink-0" />
          ) : isFileEdit ? (
            <FileCode size={11} className="text-brand shrink-0" />
          ) : (
            <Wrench size={11} className="text-muted shrink-0" />
          )}
          <span>{approval.name}</span>
        </span>
        {error && <span className="text-danger text-2xs">({error})</span>}
      </div>
      <div className="flex shrink-0 items-center gap-1.5">
        <Button
          variant="ghost"
          size="sm"
          onClick={scrollToCard}
          className="h-6 gap-1 px-2 text-2xs text-warning hover:bg-warning/20 hover:text-warning"
          title={t("詳細を確認")}
        >
          <span>{t("詳細を確認")}</span>
          <ChevronDown size={12} />
        </Button>
        <Button
          variant="default"
          size="sm"
          disabled={busy || !approval.approvalId}
          onClick={() => void decide(false)}
          className="h-6 gap-1 px-2 text-2xs border-warning/30 text-warning hover:bg-warning/20"
          title={`${t("拒否")} · Esc`}
        >
          <span>{t("拒否")}</span>
          <Kbd className="text-[9px] px-1 py-0 opacity-70">Esc</Kbd>
        </Button>
        <Button
          variant="primary"
          size="sm"
          disabled={busy || !approval.approvalId}
          onClick={() => void decide(true)}
          className="h-6 gap-1 px-2 text-2xs shadow-xs"
          title={`${t("今回のみ許可")} · ${chord("Mod", "Enter")}`}
        >
          <span>{t("今回のみ許可")}</span>
          <Kbd className="text-[9px] px-1 py-0 bg-white/20 text-white border-white/30">
            {chord("Mod", "↵")}
          </Kbd>
        </Button>
      </div>
    </div>
  );
});
