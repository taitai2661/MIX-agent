import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ShieldCheck, ShieldX } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { relativeTime } from "@/lib/format";
import {
  PENDING_APPROVALS_QUERY_KEY,
  usePendingApprovals,
} from "@/lib/queries";
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

export function ApprovalsPane() {
  const queryClient = useQueryClient();
  const pending = usePendingApprovals();
  const decide = useMutation({
    mutationFn: (input: { id: string; grant: boolean; remember: boolean }) =>
      api.decideApproval(input),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: PENDING_APPROVALS_QUERY_KEY }),
  });

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
        items.map((p: PendingApprovalView) => (
          <Card key={p.approval_id} className="border-warning">
            <div className="flex items-center justify-between gap-2">
              <strong className="font-mono text-sm font-medium text-fg">
                {p.tool_name}
              </strong>
              <Badge tone="warning" size="sm">
                {relativeTime(p.created_at)}
              </Badge>
            </div>
            <div className="my-1">
              <ApprovalDiffView
                toolName={p.tool_name}
                args={p.arguments as Record<string, unknown>}
              />
            </div>
            <Row align="end">
              {decide.isError && (
                <span className="mr-auto">
                  <ErrorBadge error={decide.error} />
                </span>
              )}
              <Button
                size="sm"
                disabled={decide.isPending}
                onClick={() =>
                  decide.mutate({
                    id: p.approval_id,
                    grant: false,
                    remember: false,
                  })
                }
              >
                <ShieldX size={12} /> {t("拒否")}
              </Button>
              <Button
                size="sm"
                disabled={decide.isPending}
                onClick={() =>
                  decide.mutate({
                    id: p.approval_id,
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
                disabled={decide.isPending}
                onClick={() =>
                  decide.mutate({
                    id: p.approval_id,
                    grant: true,
                    remember: false,
                  })
                }
              >
                {decide.isPending ? (
                  <Spinner size={12} />
                ) : (
                  <ShieldCheck size={12} />
                )}{" "}
                {t("今回のみ許可")}
              </Button>
            </Row>
          </Card>
        ))
      )}
    </Section>
  );
}
