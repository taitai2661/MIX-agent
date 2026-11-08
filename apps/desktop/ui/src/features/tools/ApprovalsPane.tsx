import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Activity, MessagesSquare, ShieldCheck, ShieldX } from "lucide-react";
import { api } from "@/lib/api";
import { t, tf } from "@/lib/i18n";
import { relativeTime } from "@/lib/format";
import {
  PENDING_APPROVALS_QUERY_KEY,
  usePendingApprovals,
} from "@/lib/queries";
import { recordUxEvent } from "@/lib/uxEvents";
import { surfaceForMode, useUi } from "@/lib/ui";
import { Card, Row, Section } from "@/components/layout";
import {
  EmptyState,
  ErrorBadge,
  ErrorBox,
  ListSkeleton,
} from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import type { PendingApprovalView } from "@/lib/types";
import { ApprovalDiffView } from "@/features/chat/ApprovalDiffView";
import {
  ApprovalCountdown,
  useApprovalRemaining,
} from "@/features/chat/ApprovalCountdown";

export function ApprovalsPane({
  location = "settings",
}: {
  /** Where the inbox is rendered, for the local UX counter only. */
  location?: "settings" | "drawer";
}) {
  const pending = usePendingApprovals();

  const items = pending.data ?? [];
  return (
    <Section>
      {pending.isPending ? (
        <ListSkeleton />
      ) : pending.isError ? (
        <ErrorBox error={pending.error} />
      ) : items.length === 0 ? (
        <EmptyState
          icon={<ShieldCheck size={22} />}
          title={t("承認待ちのアクションはありません")}
          sub={t("チャット内でもインラインで承認できます。")}
        />
      ) : (
        items.map((p) => (
          <ApprovalRow key={p.approval_id} approval={p} location={location} />
        ))
      )}
    </Section>
  );
}

function ApprovalRow({
  approval,
  location,
}: {
  approval: PendingApprovalView;
  location: "settings" | "drawer";
}) {
  const queryClient = useQueryClient();
  const setConversationId = useUi((s) => s.setConversationId);
  const setView = useUi((s) => s.setView);
  const setSurface = useUi((s) => s.setSurface);
  const setActivityRunId = useUi((s) => s.setActivityRunId);
  const setRightPanel = useUi((s) => s.setRightPanel);
  const setApprovalsOpen = useUi((s) => s.setApprovalsOpen);
  const remaining = useApprovalRemaining(approval.expires_at);
  const expired = remaining === 0;

  const decide = useMutation({
    mutationFn: (input: {
      id: string;
      grant: boolean;
      remember: boolean;
      rememberSession?: boolean;
    }) => api.decideApproval(input),
    onSuccess: (_result, input) => {
      recordUxEvent("approval_decided", {
        location,
        grant: input.grant,
        remember: input.remember,
      });
      void queryClient.invalidateQueries({
        queryKey: PENDING_APPROVALS_QUERY_KEY,
      });
    },
  });

  const disabled = decide.isPending || expired;

  // Navigating from the inbox has to land on the conversation *and* close the
  // drawer, otherwise the drawer keeps covering the pane the click just opened.
  // The activity target is optional: deciding an approval often needs the
  // tool history behind it, not just the diff in the card.
  const openThread = (withActivity: boolean) => {
    if (!approval.conversation_id) return;
    setSurface(surfaceForMode(approval.conversation_mode ?? "chat"));
    setConversationId(approval.conversation_id);
    setView("chat");
    if (withActivity) {
      setActivityRunId(approval.run_id);
      setRightPanel("activity");
    }
    setApprovalsOpen(false);
  };

  return (
    <Card className="border-warning">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <strong className="font-mono text-sm font-medium text-fg">
          {approval.tool_name}
        </strong>
        <div className="flex items-center gap-2">
          <ApprovalCountdown expiresAt={approval.expires_at} />
          <Badge tone="warning" size="sm">
            {relativeTime(approval.created_at)}
          </Badge>
        </div>
      </div>

      {/* Whose task is this? Without it, "常に許可" is granted to a prompt the
          user cannot place — the main reason an approval queue gets rubber-stamped. */}
      <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted">
        <MessagesSquare size={12} className="shrink-0 text-faint" />
        <span className="min-w-0 max-w-full truncate">
          {approval.conversation_title ?? t("不明なスレッド")}
        </span>
        <span className="text-line" aria-hidden="true">
          ·
        </span>
        <span className="font-mono text-2xs text-faint">
          {tf("Run {id}", { id: approval.run_id.slice(0, 8) })}
        </span>
        {approval.conversation_id && (
          <>
            {/* Switching the workspace first: the chat pane rejects a thread
                from the other surface and silently selects a different one. */}
            <Button
              variant="ghost"
              size="sm"
              className="h-5 px-1.5 text-2xs text-brand"
              onClick={() => openThread(false)}
            >
              <MessagesSquare size={11} aria-hidden="true" />
              {t("スレッドを開く")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="h-5 px-1.5 text-2xs text-brand"
              onClick={() => openThread(true)}
            >
              <Activity size={11} aria-hidden="true" />
              {t("アクティビティ")}
            </Button>
          </>
        )}
      </div>

      <div className="my-1">
        <ApprovalDiffView
          toolName={approval.tool_name}
          args={approval.arguments as Record<string, unknown>}
        />
      </div>

      {expired && (
        <p className="m-0 rounded-md bg-danger-soft px-2 py-1 text-xs text-danger">
          {t(
            "期限切れのため拒否されました。同じ操作が必要なら、スレッドで再実行してください。",
          )}
        </p>
      )}

      <Row align="end">
        {decide.isError && (
          <span className="mr-auto">
            <ErrorBadge error={decide.error} />
          </span>
        )}
        <Button
          size="sm"
          disabled={disabled}
          onClick={() =>
            decide.mutate({
              id: approval.approval_id,
              grant: false,
              remember: false,
            })
          }
        >
          <ShieldX size={12} /> {t("拒否")}
        </Button>
        <Button
          size="sm"
          disabled={disabled}
          onClick={() =>
            decide.mutate({
              id: approval.approval_id,
              grant: true,
              remember: false,
              rememberSession: true,
            })
          }
        >
          <ShieldCheck size={12} /> {t("このセッションで許可")}
        </Button>
        <Button
          size="sm"
          disabled={disabled}
          onClick={() =>
            decide.mutate({
              id: approval.approval_id,
              grant: true,
              remember: true,
            })
          }
        >
          {t("常に許可")}
        </Button>
        <Button
          variant="primary"
          size="sm"
          disabled={disabled}
          onClick={() =>
            decide.mutate({
              id: approval.approval_id,
              grant: true,
              remember: false,
            })
          }
        >
          {decide.isPending ? <Spinner size={12} /> : <ShieldCheck size={12} />}{" "}
          {t("今回のみ許可")}
        </Button>
      </Row>
    </Card>
  );
}
