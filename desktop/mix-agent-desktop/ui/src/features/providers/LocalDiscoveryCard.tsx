import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronDown, ChevronRight, Download, Loader2, Plus, Radar, Search, Terminal,
} from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { Card, DataTable, Field, Row, td, tdStrong } from "@/components/layout";
import { ErrorBox } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { LocalRuntimeView } from "@/lib/types";

/** Stable identity for a row: the same runtime on two ports is two rows. */
function rowKey(runtime: LocalRuntimeView): string {
  return `${runtime.host}:${runtime.port}`;
}

/**
 * Detects local inference runtimes (LM Studio, Ollama, vLLM, llama.cpp, …) and
 * turns a hit into a provider in one click.
 *
 * The scan runs only on an explicit click rather than on mount: a sweep opens a
 * connection to ten loopback ports, which is needlessly noisy on a machine
 * currently serving a large model.
 */
export function LocalDiscoveryCard() {
  const queryClient = useQueryClient();
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [imported, setImported] = useState<Record<string, boolean>>({});
  const [manualUrl, setManualUrl] = useState("");

  const scan = useQuery({
    queryKey: ["local_scan"],
    queryFn: api.scanLocalRuntimes,
    enabled: false,
    retry: false,
  });

  // A runtime the sweep does not cover — another machine on the LAN, or a
  // non-default port. It joins the same table so it is imported the same way.
  const probe = useMutation({
    mutationFn: () => api.probeLocalEndpoint(manualUrl),
  });

  const importRuntime = useMutation({
    mutationFn: (runtime: LocalRuntimeView) =>
      api.importLocalRuntime({
        slug: runtime.slug,
        kind: runtime.kind,
        label: runtime.label,
        base_url: runtime.base_url,
        models: runtime.models,
      }),
    onSuccess: (_out, runtime) => {
      setImported((prev) => ({ ...prev, [rowKey(runtime)]: true }));
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
      void queryClient.invalidateQueries({ queryKey: ["models"] });
    },
  });

  const online = (scan.data?.runtimes ?? []).filter((r) => r.online);
  const offline = (scan.data?.runtimes ?? []).filter((r) => !r.online);
  // The manually probed endpoint only joins the table after a successful probe.
  const manualResult = probe.isSuccess ? [probe.data as LocalRuntimeView] : [];
  const rows = [...online, ...offline, ...manualResult];

  const toggle = (key: string) => setExpanded((prev) => ({ ...prev, [key]: !prev[key] }));

  return (
    <>
      <Card
        title={t("ローカル LLM を検出")}
        icon={<Radar size={13} />}
        action={
          <Button
            variant="primary"
            size="sm"
            disabled={scan.isFetching}
            onClick={() => void scan.refetch()}
          >
            {scan.isFetching
              ? <Loader2 size={11} className="animate-spin" />
              : <Radar size={11} />}
            {scan.isFetching ? "検出中…" : t("検出")}
          </Button>
        }
      >
        {scan.isError && <ErrorBox error={scan.error} />}

        {scan.data && (
          <p className="m-0 text-2xs text-faint">
            {`${scan.data.hosts.join(", ")} を ${scan.data.elapsed_ms}ms で走査 — ${
              scan.data.any_online
                ? `${online.length} 件が起動中`
                : "起動中のランタイムは見つかりませんでした"
            }`}
          </p>
        )}

        {(scan.data?.occupied_ports ?? []).map((o) => (
          <p key={o.port} className="m-0 text-2xs text-warning">
            {o.note
              ? `${o.port} 番: ${o.note}`
              : `${o.port} 番は応答していますが、LLM サーバーではありません`}
          </p>
        ))}

        {!scan.data && !scan.isFetching && (
          <p className="m-0 text-xs text-muted">
            {t("LM Studio や Ollama などのローカル推論サーバーを自動で探します。")}
          </p>
        )}

        {scan.data && rows.length === 0 && (
          <p className="m-0 text-xs text-muted">
            {t("この PC で検出できるランタイムはありません。下の手動指定か、上のプリセットから追加してください。")}
          </p>
        )}

        {rows.length > 0 && (
          <DataTable
            columns={["ランタイム", "Base URL", "状態", "モデル", ""]}
            rows={rows}
            rowKey={rowKey}
          >
            {(r) => {
              const key = rowKey(r);
              const open = expanded[key] === true;
              const hasModels = r.models.length > 0;
              const isManual = probe.isSuccess && rowKey(probe.data as LocalRuntimeView) === key;
              return (
                <>
                  <td className={tdStrong}>
                    <div className="flex flex-col gap-0.5">
                      <span>{r.label}</span>
                      {r.version && (
                        <span className="font-mono text-2xs text-faint">v{r.version}</span>
                      )}
                      {!r.online && r.install_hint && (
                        <span className="font-mono text-2xs text-faint" title={r.install_hint}>
                          {r.install_hint}
                        </span>
                      )}
                    </div>
                  </td>
                  <td className={`${td} font-mono`}>{r.base_url}</td>
                  <td className={td}>
                    <div className="flex flex-col items-start gap-0.5">
                      {r.online ? (
                        <Badge tone={r.error ? "warning" : "success"} size="sm">
                          {r.latency_ms !== null ? `${r.latency_ms}ms` : "オンライン"}
                        </Badge>
                      ) : (
                        <Badge tone="warning" size="sm">{t("未起動")}</Badge>
                      )}
                      {r.error && <span className="text-2xs text-danger">{r.error}</span>}
                      {!r.online && r.launcher && (
                        <span className="flex items-center gap-1 text-2xs text-faint">
                          <Terminal size={9} />
                          <code>{r.launcher}</code>
                        </span>
                      )}
                    </div>
                  </td>
                  <td className={td}>
                    {hasModels ? (
                      <div className="flex flex-col items-start gap-1">
                        <button
                          type="button"
                          className="flex items-center gap-1 font-mono text-link hover:underline"
                          onClick={() => toggle(key)}
                          aria-expanded={open}
                        >
                          {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
                          {r.models.length}
                        </button>
                        {open && (
                          <ul className="m-0 flex max-h-52 list-none flex-col gap-1 overflow-y-auto p-0">
                            {r.models.map((m) => (
                              <li key={m.id} className="flex flex-wrap items-center gap-1.5 text-2xs">
                                <span className="font-mono text-fg">{m.id}</span>
                                {m.parameter_size && <Badge size="sm">{m.parameter_size}</Badge>}
                                {m.quantization && <Badge size="sm">{m.quantization}</Badge>}
                                {m.context_window > 0 && (
                                  <span className="text-faint">
                                    {`ctx ${Math.round(m.context_window / 1024)}k`}
                                  </span>
                                )}
                              </li>
                            ))}
                          </ul>
                        )}
                      </div>
                    ) : (
                      <span className="text-faint">—</span>
                    )}
                  </td>
                  <td className={`${td} w-24`}>
                    {r.online ? (
                      <Button
                        size="sm"
                        disabled={!hasModels || importRuntime.isPending || imported[key] === true}
                        title={
                          imported[key] === true
                            ? t("取り込み済み")
                            : hasModels
                              ? t("プロバイダーとして追加してモデルを登録")
                              : t("モデルが 1 つもロードされていません")
                        }
                        onClick={() => importRuntime.mutate(r)}
                      >
                        {imported[key] === true
                          ? <Download size={11} />
                          : <Plus size={11} />}
                        {imported[key] === true ? t("取り込み済み") : t("取り込む")}
                      </Button>
                    ) : isManual ? null : (
                      <span className="text-2xs text-faint">{t("起動してください")}</span>
                    )}
                  </td>
                </>
              );
            }}
          </DataTable>
        )}

        {importRuntime.isError && <ErrorBox error={importRuntime.error} />}
        {importRuntime.isSuccess && (
          <p className="m-0 text-xs text-success">
            {`${importRuntime.data.provider.label} を追加しました（${importRuntime.data.models.length} モデル）`}
          </p>
        )}
      </Card>

      <Card title={t("手動で URL を指定")} icon={<Search size={13} />}>
        <Field
          label="Base URL"
          hint={t("Swagger や任意のポートで起動中のサーバーも指定できます。例: http://192.168.1.10:11434/v1")}
        >
          <Input
            placeholder="http://127.0.0.1:1234/v1"
            value={manualUrl}
            onChange={(e) => setManualUrl(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && manualUrl.trim()) probe.mutate();
            }}
          />
        </Field>
        <Row>
          <Button disabled={!manualUrl.trim() || probe.isPending} onClick={() => probe.mutate()}>
            {probe.isPending
              ? <Loader2 size={12} className="animate-spin" />
              : <Search size={12} />}
            {t("テスト")}
          </Button>
          {probe.isSuccess && (
            <span className="text-2xs text-faint">
              {`${probe.data.label} / ${probe.data.models.length} モデル — 上のテーブルから追加できます`}
            </span>
          )}
        </Row>
        {probe.isError && <ErrorBox error={probe.error} />}
      </Card>
    </>
  );
}
