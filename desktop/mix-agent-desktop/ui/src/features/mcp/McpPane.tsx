import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, KeyRound, Plug, Plus, RefreshCw, Server, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { Card, DataTable, Field, Row, Section, td, tdStrong } from "@/components/layout";
import { EmptyState } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import type { McpConnectionView } from "@/lib/types";

export function McpPane() {
  const queryClient = useQueryClient();
  const connections = useQuery({ queryKey: ["mcp"], queryFn: api.listMcpConnections });
  const [draftName, setDraftName] = useState("");
  const [draftTransport, setDraftTransport] = useState("stdio");
  const [draftCommand, setDraftCommand] = useState("");
  const [draftUrl, setDraftUrl] = useState("");
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
      setNotice(`${count} MCP tool(s) available to the agent.`);
      void invalidate();
    },
    onError: (e) => setNotice(`Connect failed: ${e}`),
  });

  // Remote servers can require OAuth; send the user to the consent screen.
  const oauth = useMutation({
    mutationFn: (id: string) => api.startMcpOauth(id),
    onSuccess: async (out) => {
      setNotice("Complete sign-in in the browser; the app picks up the redirect.");
      try {
        await api.openExternal(out.authorization_url);
      } catch {
        window.open(out.authorization_url, "_blank");
      }
    },
    onError: (e) => setNotice(`OAuth failed: ${e}`),
  });

  const list = connections.data ?? [];

  return (
    <Section>
      <Card title="MCP サーバを追加" icon={<Plus size={13} />}>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label="Name">
            <Input placeholder="名前" value={draftName} onChange={(e) => setDraftName(e.target.value)} />
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
              placeholder="起動コマンド (例: npx -y @modelcontextprotocol/server-filesystem /workspace)"
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
          {create.isError && <Badge tone="danger">{String(create.error)}</Badge>}
        </Row>
      </Card>

      <Card
        title="登録済みサーバ"
        icon={<Server size={13} />}
        action={
          <Row className="flex-nowrap gap-1.5">
            <Button
              variant="primary"
              size="sm"
              disabled={connect.isPending}
              onClick={() => connect.mutate()}
            >
              <Plug size={11} /> {connect.isPending ? "接続中…" : "すべて接続"}
            </Button>
            <Button size="sm" disabled={connect.isPending} onClick={() => connect.mutate()}>
              <RefreshCw size={11} /> 再接続
            </Button>
          </Row>
        }
      >
        {toolCount !== null && <Badge tone="success" size="sm">{toolCount} tools</Badge>}
        {notice && <p className="m-0 text-xs text-muted">{notice}</p>}
        {list.length === 0 ? (
          <EmptyState
            icon={<Server size={22} />}
            title="MCP サーバはまだありません"
            sub="接続を追加して「すべて接続」を押すと agent が使える tool として公開されます。"
          />
        ) : (
          <DataTable
            columns={["名前", "Transport", "状態", "有効", "tools", ""]}
            rows={list}
            rowKey={(c: McpConnectionView) => c.id}
          >
            {(c) => (
              <>
                <td className={tdStrong}>{c.name}</td>
                <td className={td}><Badge size="sm" className="font-mono">{c.transport}</Badge></td>
                <td className={td}>
                  <Badge tone={c.status === "connected" ? "success" : "warning"} size="sm">
                    {c.status}
                  </Badge>
                </td>
                <td className={td}>
                  {c.enabled
                    ? <Badge tone="success" size="sm"><Check size={10} /></Badge>
                    : <Badge size="sm">—</Badge>}
                </td>
                <td className={td}>{c.tool_count}</td>
                <td className={`${td} w-20`}>
                  <Row align="end" className="flex-nowrap gap-0.5">
                    {c.transport === "streamable_http" && (
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        title="Sign in with OAuth"
                        aria-label="Sign in with OAuth"
                        onClick={() => oauth.mutate(c.id)}
                      >
                        <KeyRound size={13} />
                      </Button>
                    )}
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="text-danger hover:text-danger"
                      title={t("削除")}
                      aria-label={t("削除")}
                      onClick={() => remove.mutate(c.id)}
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
    </Section>
  );
}
