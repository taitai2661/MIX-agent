import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Bot, Pencil, X } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t, tf } from "@/lib/i18n";
import { relativeTime } from "@/lib/format";
import { Card, Field, Page, Row } from "@/components/layout";
import { DeleteIconButton } from "@/components/common";
import {
  EmptyState,
  ErrorBadge,
  ErrorBox,
  ListSkeleton,
} from "@/components/shared";
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
  const [editingId, setEditingId] = useState<string | null>(null);

  const resetDraft = () => {
    setEditingId(null);
    setDraftName("");
    setDraftInstructions("");
    setDraftTools([]);
  };

  const update = useMutation({
    mutationFn: () =>
      api.updateAgent({
        id: editingId!,
        name: draftName,
        instructions: draftInstructions,
        tool_ids: draftTools,
      }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["agents"] });
      resetDraft();
    },
  });

  const create = useMutation({
    mutationFn: () =>
      api.createAgent({
        name: draftName,
        instructions: draftInstructions,
        tool_ids: draftTools,
      }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["agents"] });
      setDraftName("");
      setDraftInstructions("");
      setDraftTools([]);
    },
  });

  // Agents had no delete affordance at all: the Rust command existed but was
  // never reachable, so a mistaken agent stayed in the list forever. The shared
  // `DeleteIconButton` owns the confirm dialog and renders it itself.
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteAgent(id),
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: ["agents"] }),
  });

  const list = agents.data ?? [];

  return (
    <Page
      title={t("エージェントプロファイル")}
      sub={t(
        "ツールを絞った専用エージェントを定義します。エージェントモードの実行から呼び出されます。",
      )}
    >
      <Card
        title={`${t("エージェントプロファイル")} — ${editingId ? t("編集") : t("作成")}`}
        icon={<Bot size={13} />}
      >
        <Field label={t("名前")}>
          <Input
            placeholder={t("Agent 名")}
            value={draftName}
            onChange={(e) => setDraftName(e.target.value)}
          />
        </Field>
        <Field label={t("システム指示")}>
          <Textarea
            placeholder={t("System instructions…")}
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
                  "border-line bg-surface text-muted hover:border-line-strong hover:text-fg",
                  "has-[:checked]:border-brand has-[:checked]:bg-brand-soft has-[:checked]:text-brand",
                )}
              >
                <input
                  type="checkbox"
                  className="accent-current"
                  checked={checked}
                  onChange={(e) =>
                    setDraftTools((prev) =>
                      e.target.checked
                        ? [...prev, tool.id]
                        : prev.filter((x) => x !== tool.id),
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
            disabled={!draftName || create.isPending || update.isPending}
            onClick={() => (editingId ? update.mutate() : create.mutate())}
          >
            <Bot size={12} /> {editingId ? t("保存") : t("作成")}
          </Button>
          {editingId && (
            <Button size="sm" onClick={resetDraft}>
              <X size={12} /> {t("キャンセル")}
            </Button>
          )}
          {create.isError && <ErrorBadge error={create.error} />}
          {update.isError && <ErrorBadge error={update.error} />}
        </Row>
      </Card>

      {agents.isPending ? (
        <ListSkeleton />
      ) : agents.isError ? (
        <ErrorBox error={agents.error} />
      ) : list.length === 0 ? (
        <EmptyState
          icon={<Bot size={22} />}
          title={t("エージェントプロファイル")}
          sub={t("まだ登録されていません。")}
        />
      ) : (
        <div className="flex flex-col gap-3">
          {remove.isError && <ErrorBadge error={remove.error} />}
          {list.map((a: AgentView) => (
            <Card key={a.id}>
              <Row align="between">
                <span className="text-sm font-medium text-fg">{a.name}</span>
                <Row className="flex-nowrap gap-1.5">
                  <span className="text-xs text-faint">
                    {relativeTime(a.updated_at)}
                  </span>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    title={t("編集")}
                    aria-label={tf("{name} を編集", { name: a.name })}
                    onClick={() => {
                      setEditingId(a.id);
                      setDraftName(a.name);
                      setDraftInstructions(a.instructions);
                      setDraftTools(a.tool_ids);
                    }}
                  >
                    <Pencil size={13} />
                  </Button>
                  <DeleteIconButton
                    title={t("このエージェントを削除しますか？")}
                    description={tf("{name} は元に戻せません。", {
                      name: a.name,
                    })}
                    label={tf("{name} を削除", { name: a.name })}
                    disabled={remove.isPending}
                    pending={remove.isPending && remove.variables === a.id}
                    onDelete={() => remove.mutate(a.id)}
                  />
                </Row>
              </Row>
              <Row className="gap-1">
                {a.tool_ids.map((id) => (
                  <Badge key={id} size="sm" className="font-mono">
                    {id}
                  </Badge>
                ))}
              </Row>
              <p className="m-0 whitespace-pre-wrap text-xs text-muted">
                {a.instructions}
              </p>
            </Card>
          ))}
        </div>
      )}
    </Page>
  );
}
