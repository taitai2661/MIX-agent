import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, Plug, Plus, RefreshCw, Server, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { Card, DataTable, Field, Row, Section, td, tdStrong } from "@/components/layout";
import { EmptyState, ErrorBadge, TableSkeleton } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { useConfirm } from "@/components/ui/dialog";
import { YesNo } from "@/components/ui/status";
import type { McpConnectionView } from "@/lib/types";

export function McpPane() {
  const queryClient = useQueryClient();
  const connections = useQuery({ queryKey: ["mcp"], queryFn: api.listMcpConnections });
  const [draftName, setDraftName] = useState("");
  const [draftTransport, setDraftTransport] = useState("stdio");
  const [draftCommand, setDraftCommand] = useState("");
  const [draftUrl, setDraftUrl] = useState("");
  const { confirm, dialog } = useConfirm();
  const [toolCount, setToolCount] = useState<number | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ["mcp"] });

  const create = useMutation({
    mutationFn: () =>
      api.createMcpConnection({
        name: draftName,
        transport: draftTransport,
        command: draftTransport === "stdio" ? draftCommand || null : null,
        args: null,
        url: draftTransport === "streamable_http" ? draftUrl || null : null,
        headers: null,
      }),
    onSuccess: () => {
      void invalidate();
      setDraftName("");
      setDraftCommand("");
      setDraftUrl("");
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteMcpConnection(id),
    onSuccess: () => {
      void invalidate();
    },
  });

  // Connect every configured server and republish the Tool Registry.
  const connect = useMutation({
    mutationFn: api.connectMcp,
    onSuccess: (count) => {
      setToolCount(count);
      setNotice(tf("{count} 個の MCP tool を agent から使えます。", { count }));
      void invalidate();
    },
    onError: (e) => setNotice(t("接続に失敗しました") + `: ${errorMessage(e)}`),
  });

  // Remote servers can require OAuth; send the user to the consent screen.
  const oauth = useMutation({
    mutationFn: (id: string) => api.startMcpOauth(id),
    onSuccess: async (out) => {
      setNotice(t("ブラウザでサインインを完了してください。アプリがリダイレクトを受け取ります。"));
      try {
        await api.openExternal(out.authorization_url);
      } catch {
        window.open(out.authorization_url, "_blank");
      }
    },
    onError: (e) => setNotice(t("OAuth に失敗しました") + `: ${errorMessage(e)}`),
  });

  const list = connections.data ?? [];

  return (
    <Section>
      <Card title={t("MCP サーバを追加")} icon={<Plus size={13} />}>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label={t("名前")}>
            <Input placeholder={t("名前")} value={draftName} onChange={(e) => setDraftName(e.target.value)} />
          </Field>
          <Field label="Transport">
            <Select value={draftTransport} onChange={(e) => setDraftTransport(e.target.value)}>
              <option value="stdio">stdio</option>
              <option value="streamable_http">Streamable HTTP</option>
            </Select>
          </Field>
        </div>
        {draftTransport === "stdio" ? (
          <Field label="Command">
            <Input
              placeholder={t("起動コマンド (例: npx -y @modelcontextprotocol/server-filesystem /workspace)")}
              value={draftCommand}
              onChange={(e) => setDraftCommand(e.target.value)}
            />
          </Field>
        ) : (
          <Field label="URL">
            <Input
              placeholder="https://mcp.example.com/mcp"
              value={draftUrl}
              onChange={(e) => setDraftUrl(e.target.value)}
            />
          </Field>
        )}
        <Row>
          <Button
            variant="primary"
            disabled={!draftName || create.isPending}
            onClick={() => create.mutate()}
          >
            <Plus size={12} /> {t("作成")}
          </Button>
          {create.isError && <ErrorBadge error={create.error} />}
        </Row>
      </Card>

      <Card
        title={t("登録済みサーバ")}
        icon={<Server size={13} />}
        action={
          <Row className="flex-nowrap gap-1.5">
            <Button
              variant="primary"
              size="sm"
              disabled={connect.isPending}
              onClick={() => connect.mutate()}
            >
              <Plug size={11} /> {connect.isPending ? t("接続中…") : t("すべて接続")}
            </Button>
            <Button size="sm" disabled={connect.isPending} onClick={() => connect.mutate()}>
              <RefreshCw size={11} /> {t("再接続")}
            </Button>
          </Row>
        }
      >
        {toolCount !== null && (
          <Badge tone="success" size="sm">{`${toolCount} ${t("件")}`}</Badge>
        )}
        {notice && <p className="m-0 text-xs text-muted">{notice}</p>}
        {connections.isPending ? (
          <TableSkeleton />
        ) : list.length === 0 ? (
          <EmptyState
            icon={<Server size={22} />}
            title={t("MCP サーバはまだありません")}
            sub={t("接続を追加して「すべて接続」を押すと agent が使える tool として公開されます。")}
          />
        ) : (
          <DataTable
            columns={[t("名前"), "Transport", t("状態"), t("有効"), t("ツール数"), ""]}
            rows={list}
            rowKey={(c: McpConnectionView) => c.id}
          >
            {(c) => (
              <>
                <td className={tdStrong}>{c.name}</td>
                <td className={td}><Badge size="sm" className="font-mono">{c.transport}</Badge></td>
                <td className={td}>
                  <Badge tone={c.status === "connected" ? "success" : "warning"} size="sm">
                    {c.status === "connected" ? t("接続済み") : t("未接続")}
                  </Badge>
                </td>
                <td className={td}>
                  <YesNo value={c.enabled} onLabel={t("有効")} offLabel={t("無効")} />
                </td>
                <td className={td}>{c.tool_count}</td>
                <td className={`${td} w-20`}>
                  <Row align="end" className="flex-nowrap gap-0.5">
                    {c.transport === "streamable_http" && (
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        title={t("OAuth でサインイン")}
                        aria-label={t("OAuth でサインイン")}
                        onClick={() => oauth.mutate(c.id)}
                      >
                        <KeyRound size={13} />
                      </Button>
                    )}
                    <Button
                      variant="ghost-danger"
                      size="icon-sm"
                      title={t("削除")}
                      aria-label={t("削除")}
                      onClick={async () => {
                        // Deleting an MCP server used to be the one destructive
                        // action with no confirmation at all, while the provider
                        // and thread deletions right next to it had one.
                        if (await confirm({
                          title: t("MCP 接続を削除しますか？"),
                          description: c.name,
                          confirmLabel: t("削除"),
                          tone: "danger",
                        })) {
                          remove.mutate(c.id);
                        }
                      }}
                    >
                      <Trash2 size={13} />
                    </Button>
                  </Row>
                </td>
              </>
            )}
          </DataTable>
        )}
      </Card>
      {dialog}
    </Section>
  );
}
