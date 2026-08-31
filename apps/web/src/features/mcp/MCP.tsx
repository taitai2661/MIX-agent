import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { useQueryClient } from "@tanstack/react-query";
import { FileText, Globe, Plus } from "lucide-react";
import { useState } from "react";

import { Empty, ErrorBox, Field, Title, useRows } from "@/components/shared";

export function MCP() {
  const rows = useRows("/mcp/connections"),
    qc = useQueryClient();
  const [open, setOpen] = useState(false),
    [editing, setEditing] = useState<Row | null>(null),
    [transport, setTransport] = useState("http"),
    [error, setError] = useState<unknown>(null),
    [notice, setNotice] = useState(""),
    [template, setTemplate] = useState(false);
  return (
    <>
      <Title
        title="MCP"
        sub="外部サービスを、Agentが使えるToolとして接続します。"
        action={
          <Button
            onClick={() => {
              setEditing(null);
              setTemplate(false);
              setOpen(!open);
            }}
          >
            <Plus size={16} />
            接続を追加
          </Button>
        }
      />
      <div className="notice">
        Remote接続はHTTPS・Bearer/カスタムHeaderに対応。OAuth必須の接続は未対応です。接続先を「一般・通信」で許可してください。
      </div>
      <ErrorBox error={error || rows.error} />
      {notice && <p className="success">{notice}</p>}
      <div className="template-row">
        <Button
          variant="outline"
          onClick={() => {
            setTemplate(true);
            setEditing(null);
            setTransport("stdio");
            setOpen(true);
          }}
        >
          <FileText size={16} />
          Filesystemテンプレート
        </Button>
      </div>
      {open && (
        <form
          className="card form-grid"
          key={editing?.id || String(template)}
          onSubmit={async (e) => {
            e.preventDefault();
            const f = new FormData(e.currentTarget);
            try {
              await api(
                "/mcp/connections" + (editing ? "/" + editing.id : ""),
                editing ? "PATCH" : "POST",
                {
                  name: f.get("name"),
                  transport,
                  url: f.get("url") || "",
                  command: f.get("command") || "",
                  args: JSON.parse(String(f.get("args") || "[]")),
                  credentials: f.get("credentials")
                    ? JSON.parse(String(f.get("credentials")))
                    : null,
                },
              );
              setOpen(false);
              setTemplate(false);
              qc.invalidateQueries({ queryKey: ["/mcp/connections"] });
            } catch (e) {
              setError(e);
            }
          }}
        >
          <Field label="名前">
            <input
              name="name"
              required
              defaultValue={
                editing?.data.name || (template ? "Filesystem" : "")
              }
            />
          </Field>
          <Field label="接続方式">
            <select
              value={transport}
              onChange={(e) => setTransport(e.target.value)}
            >
              <option value="http">Streamable HTTP</option>
              <option value="stdio">stdio</option>
            </select>
          </Field>
          {transport === "http" ? (
            <Field label="URL">
              <input
                name="url"
                required
                type="url"
                placeholder="https://example.com/mcp"
                defaultValue={editing?.data.url}
              />
            </Field>
          ) : (
            <>
              <Field label="インストール済み実行ファイル">
                <input
                  name="command"
                  required
                  defaultValue={
                    editing?.data.command ||
                    (template
                      ? "/packages/node_modules/.bin/mcp-server-filesystem"
                      : "")
                  }
                />
              </Field>
              <Field label="引数（JSON配列）">
                <input
                  name="args"
                  defaultValue={
                    editing
                      ? JSON.stringify(editing.data.args)
                      : template
                        ? '["/shared"]'
                        : "[]"
                  }
                />
              </Field>
            </>
          )}
          <Field
            label="Secret（JSON）"
            hint={
              '例: {"headers":{"Authorization":"Bearer ..."}} または {"env":{"TOKEN":"..."}}'
            }
          >
            <textarea
              name="credentials"
              defaultValue={editing ? "" : "{}"}
              placeholder="空欄で保持、{}でSecretを削除"
              autoComplete="off"
            />
          </Field>
          <div className="form-actions">
            <Button>保存</Button>
            <Button
              variant="ghost"
              type="button"
              onClick={() => setOpen(false)}
            >
              キャンセル
            </Button>
          </div>
        </form>
      )}
      {rows.data?.map((r) => (
        <div className="card" key={r.id}>
          <h3>
            <Globe size={18} />
            {r.data.name}
          </h3>
          <p>
            {r.data.transport} · {r.data.url || r.data.command}
          </p>
          <small>{r.data.enabled ? "有効" : "切断済み"}</small>
          <div className="form-actions">
            <Button
              variant="ghost"
              onClick={() => {
                setEditing(r);
                setTemplate(false);
                setTransport(r.data.transport);
                setOpen(true);
              }}
            >
              編集
            </Button>
            {r.data.command ===
              "/packages/node_modules/.bin/mcp-server-filesystem" && (
              <Button
                variant="outline"
                onClick={async () => {
                  setError(null);
                  try {
                    await api("/mcp/connections/" + r.id + "/install", "POST");
                    setNotice(
                      "パッケージを導入しました。接続・Tool取得を実行してください。",
                    );
                  } catch (e) {
                    setError(e);
                  }
                }}
              >
                パッケージを導入
              </Button>
            )}
            {[
              ["test", "接続テスト"],
              ["sync", "接続・Tool取得"],
              ["disconnect", "切断"],
            ].map(([a, label]) => (
              <Button
                key={a}
                variant="outline"
                onClick={async () => {
                  setError(null);
                  try {
                    const result = await api(
                      "/mcp/connections/" + r.id + "/" + a,
                      "POST",
                    );
                    setNotice(
                      result.tools
                        ? result.tools.length + "件のToolを確認しました"
                        : "切断しました",
                    );
                    qc.invalidateQueries({ queryKey: ["/mcp/connections"] });
                    qc.invalidateQueries({ queryKey: ["/tools"] });
                  } catch (e) {
                    setError(e);
                  }
                }}
              >
                {label}
              </Button>
            ))}
          </div>
        </div>
      ))}
      {!rows.data?.length && !open && (
        <Empty>MCP接続を追加すると、Tool Registryに登録できます。</Empty>
      )}
    </>
  );
}
