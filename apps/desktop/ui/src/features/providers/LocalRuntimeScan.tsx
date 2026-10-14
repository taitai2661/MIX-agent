import { useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ChevronDown, ChevronRight, Download, Plus, Radar, Search, Terminal,
} from "lucide-react";
import { api } from "@/lib/api";
import { t, tf } from "@/lib/i18n";
import { DataTable, Field, Row, Section, td, tdStrong } from "@/components/layout";
import { ErrorBadge, ErrorBox } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/label";
import { Spinner } from "@/components/ui/spinner";
import { StatusDot } from "@/components/ui/status";
import { MODELS_QUERY_KEY, PROVIDERS_QUERY_KEY } from "@/lib/queries";
import type { LocalRuntimeView } from "@/lib/types";

/** Stable identity for a row: the same runtime on two ports is two rows. */
function rowKey(runtime: LocalRuntimeView): string {
  return `${runtime.host}:${runtime.port}`;
}

export interface LocalRuntimeScanProps {
  /** Shown before anything has been scanned. */
  intro?: ReactNode;
  /** Shown after a scan that found nothing — the caller's chance to point at
   * an alternative (the onboarding wizard uses it to suggest the cloud tab). */
  emptyHint?: ReactNode;
  /** Append the manual Base URL field. Off in the wizard, where it is a second
   * way to do something the first tab already covers and only invites typos. */
  showManualProbe?: boolean;
  /** Called after a runtime is imported, with the provider it created. */
  onImported?: (providerLabel: string) => void;
}

/**
 * Detects local inference runtimes (LM Studio, Ollama, vLLM, llama.cpp, …) and
 * turns a hit into a provider in one click.
 *
 * The scan runs only on an explicit click rather than on mount: a sweep opens a
 * connection to ten loopback ports, which is needlessly noisy on a machine
 * currently serving a large model.
 *
 * Shared by Settings → Providers and the first-run wizard. It owns no chrome of
 * its own beyond the scan trigger and the manual field, so a caller can wrap it
 * in whatever surface it needs.
 */
export function LocalRuntimeScan({
  intro,
  emptyHint,
  showManualProbe = false,
  onImported,
}: LocalRuntimeScanProps) {
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
    onSuccess: (out, runtime) => {
      setImported((prev) => ({ ...prev, [rowKey(runtime)]: true }));
      void queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY });
      void queryClient.invalidateQueries({ queryKey: MODELS_QUERY_KEY });
      onImported?.(out.provider.label);
    },
  });

  const online = (scan.data?.runtimes ?? []).filter((r) => r.online);
  const offline = (scan.data?.runtimes ?? []).filter((r) => !r.online);
  // The manually probed endpoint only joins the table after a successful probe.
  const manualResult = probe.isSuccess ? [probe.data as LocalRuntimeView] : [];
  const rows = [...online, ...offline, ...manualResult];

  const toggle = (key: string) => setExpanded((prev) => ({ ...prev, [key]: !prev[key] }));

  return (
    <Section>
      <Row>
        <Button
          variant="primary"
          disabled={scan.isFetching}
          onClick={() => void scan.refetch()}
        >
          {scan.isFetching
            ? <Spinner size={12} />
            : <Radar size={12} />}
          {scan.isFetching ? t("検出中…") : t("検出")}
        </Button>
        {intro && !scan.data && !scan.isFetching && (
          <span className="text-xs text-muted">{intro}</span>
        )}
      </Row>

      {scan.isError && <ErrorBox error={scan.error} />}

      {scan.data && (
        <p className="m-0 text-xs text-faint">
          {tf("{hosts} を {ms}ms で走査 — {found}", {
            hosts: scan.data.hosts.join(", "),
            ms: scan.data.elapsed_ms,
            found: scan.data.any_online
              ? tf("{n} 件が起動中", { n: online.length })
              : t("起動中のランタイムは見つかりませんでした"),
          })}
        </p>
      )}

      {(scan.data?.occupied_ports ?? []).map((o) => (
        <p key={o.port} className="m-0 text-xs text-warning">
          {o.note
            ? tf("{port} 番: {note}", { port: o.port, note: o.note })
            : tf("{port} 番は応答していますが、LLM サーバーではありません", { port: o.port })}
        </p>
      ))}

      {scan.data && rows.length === 0 && (
        <p className="m-0 text-xs text-muted">{emptyHint}</p>
      )}

      {rows.length > 0 && (
        <DataTable
          columns={[t("ランタイム"), "Base URL", t("状態"), t("モデル"), ""]}
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
                      <span className="font-mono text-xs text-faint">v{r.version}</span>
                    )}
                    {!r.online && r.install_hint && (
                      <span className="font-mono text-xs text-faint" title={r.install_hint}>
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
                        {t("オンライン")}
                        {r.latency_ms !== null && (
                          <span className="font-mono text-faint">{`${r.latency_ms}ms`}</span>
                        )}
                      </Badge>
                    ) : (
                      <StatusDot tone="warning" label={t("未起動")} />
                    )}
                    {r.error && <ErrorBadge error={r.error} />}
                    {!r.online && r.launcher && (
                      <span className="flex items-center gap-1 text-xs text-faint">
                        <Terminal size={12} />
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
                        className="flex items-center gap-1 rounded-sm px-1 font-mono text-brand hover:bg-hover hover:underline"
                        onClick={() => toggle(key)}
                        aria-expanded={open}
                      >
                        {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                        {r.models.length}
                      </button>
                      {open && (
                        <ul className="m-0 flex max-h-52 list-none flex-col gap-1 overflow-y-auto p-0">
                          {r.models.map((m) => (
                            <li key={m.id} className="flex flex-wrap items-center gap-1.5 text-xs">
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
                        ? <Download size={12} />
                        : <Plus size={12} />}
                      {imported[key] === true ? t("取り込み済み") : t("取り込む")}
                    </Button>
                  ) : isManual ? null : (
                    <span className="text-xs text-faint">{t("起動してください")}</span>
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
          {tf("{label} を追加しました（{n} モデル）", {
            label: importRuntime.data.provider.label,
            n: importRuntime.data.models.length,
          })}
        </p>
      )}

      {showManualProbe && (
        <>
          <Separator />
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
                ? <Spinner size={12} />
                : <Search size={12} />}
              {t("テスト")}
            </Button>
            {probe.isSuccess && (
              <span className="text-xs text-faint">
                {tf("{label} / {n} モデル — 上のテーブルから追加できます", {
                  label: probe.data.label,
                  n: probe.data.models.length,
                })}
              </span>
            )}
          </Row>
          {probe.isError && <ErrorBox error={probe.error} />}
        </>
      )}
    </Section>
  );
}