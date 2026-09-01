import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Box,
  Check,
  ChevronRight,
  Download,
  FileText,
  Globe,
  KeyRound,
  LoaderCircle,
  Network,
  PackageOpen,
  Plus,
  Search,
  ShieldCheck,
  X,
} from "lucide-react";
import { useDeferredValue, useState } from "react";

import { Empty, ErrorBox, Field, Title, useRows } from "@/components/shared";

export function MCP() {
  const rows = useRows("/mcp/connections"),
    qc = useQueryClient();
  const [open, setOpen] = useState(false),
    [editing, setEditing] = useState<Row | null>(null),
    [transport, setTransport] = useState("http"),
    [error, setError] = useState<unknown>(null),
    [notice, setNotice] = useState(""),
    [template, setTemplate] = useState(false),
    [query, setQuery] = useState(""),
    [selected, setSelected] = useState<any>(null),
    [networkCapability, setNetworkCapability] = useState("none"),
    [installing, setInstalling] = useState(false);
  const deferredQuery = useDeferredValue(query);
  const catalog = useQuery({
    queryKey: ["/mcp/registry", deferredQuery],
    queryFn: () =>
      api<any>("/mcp/registry?q=" + encodeURIComponent(deferredQuery)),
  });
  const installed = new Set(
    rows.data?.map((row) => row.data.registry_id).filter(Boolean) || [],
  );
  const selectForInstall = (server: any) => {
    setError(null);
    setNotice("");
    setNetworkCapability("none");
    setSelected(server);
  };
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
      <ErrorBox error={error || rows.error || catalog.error} />
      {notice && (
        <p className="success">
          <Check size={16} />
          {notice}
        </p>
      )}
      <section className="mcp-store" aria-labelledby="mcp-store-title">
        <div className="mcp-store-intro">
          <div>
            <p className="eyebrow">MCP STORE</p>
            <h2 id="mcp-store-title">Toolを追加</h2>
            <p>
              公式Registryから選ぶだけ。ローカルMCPは専用コンテナで隔離されます。
            </p>
          </div>
          <span className="mcp-safe-badge">
            <ShieldCheck size={16} />
            既定でネットワークなし
          </span>
        </div>
        <label className="mcp-store-search">
          <Search size={18} />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="名前やサービスで検索"
            aria-label="MCP Registryを検索"
          />
          {query && (
            <button
              type="button"
              aria-label="検索をクリア"
              onClick={() => setQuery("")}
            >
              <X size={16} />
            </button>
          )}
        </label>
        {catalog.isLoading && (
          <div className="mcp-store-state">
            <LoaderCircle className="spin" size={22} />
            Registryを読み込んでいます
          </div>
        )}
        {!catalog.isLoading && !catalog.data?.servers?.length && (
          <div className="mcp-store-state">
            <PackageOpen size={24} />
            <strong>該当するMCPがありません</strong>
            <span>別の名前やサービス名で検索してください。</span>
          </div>
        )}
        <div className="mcp-store-grid">
          {catalog.data?.servers?.map((server: any) => (
            <article
              className={
                "card mcp-store-card" +
                (installed.has(server.registry_id) ? " is-installed" : "")
              }
              key={server.registry_id}
            >
              <div className="mcp-store-heading">
                {server.selected?.kind === "remote" ? (
                  <Globe size={19} />
                ) : (
                  <Box size={19} />
                )}
                <div>
                  <h3>{server.title}</h3>
                  <small>
                    {server.registry_id} · v{server.version}
                  </small>
                </div>
              </div>
              <p>{server.description || "説明はありません"}</p>
              <div className="mcp-card-meta">
                <span className="tag">{server.selected?.kind || "未対応"}</span>
                {server.selected?.kind !== "remote" && server.selected && (
                  <span>
                    <ShieldCheck size={13} />
                    Docker隔離
                  </span>
                )}
              </div>
              <Button
                variant={
                  installed.has(server.registry_id) ? "ghost" : "outline"
                }
                disabled={!server.selected || installed.has(server.registry_id)}
                onClick={() => selectForInstall(server)}
              >
                {installed.has(server.registry_id) ? (
                  <Check size={15} />
                ) : (
                  <Download size={15} />
                )}
                {installed.has(server.registry_id)
                  ? "導入済み"
                  : server.selected
                    ? "内容を確認"
                    : "未対応"}
                {server.selected && !installed.has(server.registry_id) && (
                  <ChevronRight size={15} />
                )}
              </Button>
            </article>
          ))}
        </div>
      </section>
      {selected && (
        <div
          className="mcp-install-backdrop"
          role="presentation"
          onMouseDown={(event) => {
            if (event.target === event.currentTarget && !installing)
              setSelected(null);
          }}
        >
          <form
            className="card form-grid mcp-install-panel"
            aria-labelledby="mcp-install-title"
            onSubmit={async (event) => {
              event.preventDefault();
              const form = new FormData(event.currentTarget);
              const domains = String(form.get("domains") || "")
                .split("\n")
                .map((value) => value.trim())
                .filter(Boolean);
              const secrets: Record<string, string> = {};
              selected.selected?.environment
                ?.filter((item: any) => item.secret)
                .forEach((item: any) => {
                  const value = String(form.get("secret:" + item.name) || "");
                  if (value) secrets[item.name] = value;
                });
              setInstalling(true);
              setError(null);
              try {
                const connection = await api<any>(
                  "/mcp/registry/install",
                  "POST",
                  {
                    registry_id: selected.registry_id,
                    version: selected.version,
                    network_capability: form.get("network"),
                    allowed_domains: domains,
                    configuration: {},
                    secrets: Object.keys(secrets).length ? secrets : null,
                  },
                );
                let toolCount: number | null = null;
                try {
                  const synced = await api<any>(
                    "/mcp/connections/" + connection.id + "/sync",
                    "POST",
                  );
                  toolCount = synced.tools?.length ?? 0;
                } catch {
                  /* 接続は導入済み一覧から再試行できる */
                }
                setSelected(null);
                setNotice(
                  toolCount === null
                    ? "導入しました。接続確認は導入済み一覧から再実行できます。"
                    : `導入が完了し、${toolCount}件のToolを利用できるようになりました。`,
                );
                qc.invalidateQueries({ queryKey: ["/mcp/connections"] });
                qc.invalidateQueries({ queryKey: ["/tools"] });
              } catch (caught) {
                setError(caught);
              } finally {
                setInstalling(false);
              }
            }}
          >
            <div className="mcp-install-header">
              <div className="mcp-install-icon">
                {selected.selected?.kind === "remote" ? (
                  <Globe size={24} />
                ) : (
                  <Box size={24} />
                )}
              </div>
              <div>
                <p className="eyebrow">INSTALL MCP</p>
                <h2 id="mcp-install-title">{selected.title}</h2>
                <p>
                  {selected.registry_id} · v{selected.version}
                </p>
              </div>
              <button
                type="button"
                className="icon-button"
                aria-label="閉じる"
                disabled={installing}
                onClick={() => setSelected(null)}
              >
                <X size={19} />
              </button>
            </div>
            <p className="mcp-install-description">
              {selected.description || "説明はありません"}
            </p>
            <div className="mcp-install-summary">
              <span>
                <PackageOpen size={16} />
                {selected.selected?.kind?.toUpperCase()}
              </span>
              <span>
                <ShieldCheck size={16} />
                {selected.selected?.kind === "remote"
                  ? "リクエスト単位で接続"
                  : "専用Dockerコンテナ"}
              </span>
              <span>
                <KeyRound size={16} />
                Secretは実行時のみ注入
              </span>
            </div>
            <fieldset className="mcp-network-options">
              <legend>
                <Network size={16} />
                実行時ネットワーク
              </legend>
              {[
                {
                  value: "none",
                  title: "なし",
                  description: "外部通信を許可しません（推奨）",
                },
                {
                  value: "restricted",
                  title: "指定ドメインのみ",
                  description: "必要な接続先だけを許可します",
                },
                {
                  value: "public_web",
                  title: "公開Web",
                  description: "公開IPのHTTP/HTTPSを許可します",
                },
              ].map((option) => (
                <label
                  key={option.value}
                  className={
                    networkCapability === option.value ? "is-selected" : ""
                  }
                >
                  <input
                    type="radio"
                    name="network"
                    value={option.value}
                    checked={networkCapability === option.value}
                    onChange={() => setNetworkCapability(option.value)}
                  />
                  <span>
                    <strong>{option.title}</strong>
                    <small>{option.description}</small>
                  </span>
                </label>
              ))}
            </fieldset>
            {networkCapability === "restricted" && (
              <Field
                label="許可ドメイン"
                hint="1行に1ホスト。localhost、private、metadata宛は常に拒否されます。"
              >
                <textarea
                  name="domains"
                  required
                  placeholder={"api.example.com\ncdn.example.com"}
                />
              </Field>
            )}
            {selected.selected?.environment?.filter((item: any) => item.secret)
              .length > 0 && (
              <div className="mcp-secret-section">
                <h3>
                  <KeyRound size={16} />
                  必要なSecret
                </h3>
                <p>暗号化してMCP単位で保存し、コンテナ起動時だけ注入します。</p>
                {selected.selected.environment
                  .filter((item: any) => item.secret)
                  .map((item: any) => (
                    <Field
                      key={item.name}
                      label={item.name}
                      hint={item.description}
                    >
                      <input
                        name={"secret:" + item.name}
                        type="password"
                        required={item.required}
                        autoComplete="off"
                        placeholder={item.required ? "必須" : "任意"}
                      />
                    </Field>
                  ))}
              </div>
            )}
            <div className="mcp-install-actions">
              <Button
                type="button"
                variant="ghost"
                disabled={installing}
                onClick={() => setSelected(null)}
              >
                キャンセル
              </Button>
              <Button disabled={installing}>
                {installing ? (
                  <>
                    <LoaderCircle className="spin" size={16} />
                    安全にインストール中…
                  </>
                ) : (
                  <>
                    <Download size={16} />
                    インストール
                  </>
                )}
              </Button>
            </div>
          </form>
        </div>
      )}
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
            {!r.data.registry_id && (
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
            )}
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
            {r.data.transport === "http" && (
              <Button
                variant="outline"
                onClick={async () => {
                  try {
                    const result = await api<any>(
                      "/mcp/connections/" + r.id + "/oauth/start",
                      "POST",
                      { client_id: "", client_secret: "", scopes: [] },
                    );
                    window.location.assign(result.authorization_url);
                  } catch (caught) {
                    setError(caught);
                  }
                }}
              >
                OAuthで認証
              </Button>
            )}
            {r.data.registry_id && (
              <Button
                variant="outline"
                onClick={async () => {
                  try {
                    await api("/mcp/connections/" + r.id + "/update", "POST");
                    setNotice("更新を確認しました");
                    qc.invalidateQueries({ queryKey: ["/mcp/connections"] });
                  } catch (caught) {
                    setError(caught);
                  }
                }}
              >
                更新を確認
              </Button>
            )}
            {r.data.state === "uninstalled_data_retained" && (
              <Button
                onClick={async () => {
                  try {
                    await api(
                      "/mcp/connections/" + r.id + "/reinstall",
                      "POST",
                    );
                    qc.invalidateQueries({ queryKey: ["/mcp/connections"] });
                  } catch (caught) {
                    setError(caught);
                  }
                }}
              >
                保持データから再導入
              </Button>
            )}
            {r.data.runtime?.driver &&
              r.data.runtime.driver !== "remote" &&
              ["start", "stop", "restart"].map((action) => (
                <Button
                  key={action}
                  variant="ghost"
                  onClick={async () => {
                    try {
                      await api(
                        "/mcp/connections/" + r.id + "/runtime/" + action,
                        "POST",
                      );
                      qc.invalidateQueries({ queryKey: ["/mcp/connections"] });
                    } catch (caught) {
                      setError(caught);
                    }
                  }}
                >
                  {action === "start"
                    ? "起動"
                    : action === "stop"
                      ? "停止"
                      : "再起動"}
                </Button>
              ))}
            <Button
              variant="destructive"
              onClick={async () => {
                const complete = window.confirm(
                  "専用データvolumeも完全削除しますか？\nキャンセルするとデータを保持して接続だけ削除します。",
                );
                const volume = r.data.runtime?.volume || "";
                if (
                  complete &&
                  !window.confirm(
                    "完全削除すると復旧できません。続行しますか？\n" + volume,
                  )
                )
                  return;
                try {
                  await api("/mcp/connections/" + r.id + "/uninstall", "POST", {
                    delete_volume: complete,
                    confirm_volume: complete ? volume : "",
                  });
                  qc.invalidateQueries({ queryKey: ["/mcp/connections"] });
                  qc.invalidateQueries({ queryKey: ["/tools"] });
                } catch (caught) {
                  setError(caught);
                }
              }}
            >
              アンインストール
            </Button>
          </div>
        </div>
      ))}
      {!rows.data?.length && !open && (
        <Empty>MCP接続を追加すると、Tool Registryに登録できます。</Empty>
      )}
    </>
  );
}
