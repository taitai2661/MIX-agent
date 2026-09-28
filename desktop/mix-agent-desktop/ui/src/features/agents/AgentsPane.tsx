import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Bot } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { relativeTime } from "@/lib/format";
import { Card, Field, Page, Row } from "@/components/layout";
import { EmptyState } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Textarea } from "@/components/ui/input";
import type { AgentView, ToolView } from "@/lib/types";

export function AgentsPane() {
  const queryClient = useQueryClient();
  const agents = useQuery({ queryKey: ["agents"], queryFn: api.listAgents });
  const tools = useQuery({ queryKey: ["tools"], queryFn: api.listTools });
  const [draftName, setDraftName] = useState("");
  const [draftInstructions, setDraftInstructions] = useState("");
  const [draftTools, setDraftTools] = useState<string[]>([]);

  const create = useMutation({
    mutationFn: () =>
      api.createAgent({ name: draftName, instructions: draftInstructions, tool_ids: draftTools }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["agents"] });
      setDraftName("");
      setDraftInstructions("");
      setDraftTools([]);
    },
  });

  const list = agents.data ?? [];

  return (
    <Page title={t("エージェント")} sub="ツールを絞った専用エージェントを定義します。">
      <Card title={`${t("エージェント")} — ${t("作成")}`} icon={<Bot size={13} />}>
        <Field label="Name">
          <Input
            placeholder="Agent 名"
            value={draftName}
            onChange={(e) => setDraftName(e.target.value)}
          />
        </Field>
        <Field label="System instructions">
          <Textarea
            placeholder="System instructions…"
            value={draftInstructions}
            onChange={(e) => setDraftInstructions(e.target.value)}
          />
        </Field>
        <Row className="gap-1.5">
          {(tools.data ?? []).map((tool: ToolView) => {
            const checked = draftTools.includes(tool.id);
            return (
              <label
                key={tool.id}
                className={cn(
                  "inline-flex cursor-pointer items-center gap-1.5 rounded-md border px-2 py-1",
                  "text-xs transition-colors duration-100",
                  checked
                    ? "border-brand bg-brand-soft text-brand"
                    : "border-line bg-surface text-muted hover:border-line-strong hover:text-fg",
                )}
              >
                <input
                  type="checkbox"
                  className="accent-current"
                  checked={checked}
                  onChange={(e) =>
                    setDraftTools((prev) =>
                      e.target.checked ? [...prev, tool.id] : prev.filter((x) => x !== tool.id),
                    )
                  }
                />
                {tool.name}
              </label>
            );
          })}
        </Row>
        <Row>
          <Button
            variant="primary"
            disabled={!draftName || create.isPending}
            onClick={() => create.mutate()}
          >
            <Bot size={12} /> {t("作成")}
          </Button>
          {create.isError && <Badge tone="danger">{String(create.error)}</Badge>}
        </Row>
      </Card>

      {list.length === 0 ? (
        <EmptyState icon={<Bot size={22} />} title={t("エージェント")} sub="まだ登録されていません。" />
      ) : (
        <div className="flex flex-col gap-3">
          {list.map((a: AgentView) => (
            <Card key={a.id}>
              <Row align="between">
                <strong className="text-sm text-fg">{a.name}</strong>
                <span className="text-2xs text-faint">{relativeTime(a.updated_at)}</span>
              </Row>
              <Row className="gap-1">
                {a.tool_ids.map((id) => (
                  <Badge key={id} size="sm" className="font-mono">{id}</Badge>
                ))}
              </Row>
              <p className="m-0 whitespace-pre-wrap text-xs text-muted">{a.instructions}</p>
            </Card>
          ))}
        </div>
      )}
    </Page>
  );
}
