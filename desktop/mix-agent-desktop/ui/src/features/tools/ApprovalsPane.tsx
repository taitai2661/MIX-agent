import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ShieldCheck, ShieldX } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { relativeTime } from "@/lib/format";
import { PENDING_APPROVALS_QUERY_KEY, usePendingApprovals } from "@/lib/queries";
import { Card, Row, Section } from "@/components/layout";
import { EmptyState, ListSkeleton } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { PendingApprovalView } from "@/lib/types";

export function ApprovalsPane() {
  const queryClient = useQueryClient();
  const pending = usePendingApprovals();
  const decide = useMutation({
    mutationFn: (input: { id: string; grant: boolean; remember: boolean }) =>
      api.decideApproval(input),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: PENDING_APPROVALS_QUERY_KEY }),
  });

  const items = pending.data ?? [];
  return (
    <Section>
      {pending.isPending ? (
        <ListSkeleton />
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
              <strong className="font-mono text-sm text-fg">{p.tool_name}</strong>
              <Badge tone="warning" size="sm">{relativeTime(p.created_at)}</Badge>
            </div>
            <pre className="m-0 max-h-44 overflow-auto rounded-md bg-inset p-2 font-mono text-xs leading-4 text-muted">
              {JSON.stringify(p.arguments, null, 2).slice(0, 1200)}
            </pre>
            <Row align="end">
              <Button
                size="sm"
                onClick={() => decide.mutate({ id: p.approval_id, grant: false, remember: false })}
              >
                <ShieldX size={11} /> {t("拒否")}
              </Button>
              <Button
                size="sm"
                onClick={() => decide.mutate({ id: p.approval_id, grant: true, remember: true })}
              >
                {t("常に許可")}
              </Button>
              <Button
                variant="primary"
                size="sm"
                onClick={() => decide.mutate({ id: p.approval_id, grant: true, remember: false })}
              >
                <ShieldCheck size={11} /> {t("今回のみ許可")}
              </Button>
            </Row>
          </Card>
        ))
      )}
    </Section>
  );
}
