import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Info, Search, Sparkles, Zap } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { formatBytes, formatContext, shortModelId } from "@/lib/format";
import { Section, SectionLabel } from "@/components/layout";
import { EmptyState, ErrorBox, ListSkeleton } from "@/components/shared";
import { SearchEmptyState } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Select, SearchField } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { YesNo } from "@/components/ui/status";
import { useUi } from "@/lib/ui";
import { MODELS_QUERY_KEY, useModels, useProviders } from "@/lib/queries";
import type { ModelView } from "@/lib/types";

function CapabilityRow({ model }: { model: ModelView }) {
  const caps: { label: string; on: boolean }[] = [
    { label: t("Vision"), on: model.supports_vision },
    { label: t("ツール"), on: model.supports_tool_calling },
    { label: t("推論"), on: model.supports_reasoning },
  ];
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {caps.map((cap) => (
        <span key={cap.label} className="inline-flex items-center gap-1">
          <span className="text-2xs text-faint">{cap.label}</span>
          <YesNo value={cap.on} />
        </span>
      ))}
    </div>
  );
}

export function ModelsPane() {
  const queryClient = useQueryClient();
  const openSettings = useUi((s) => s.openSettings);
  const models = useModels();
  const providers = useProviders();
  const [query, setQuery] = useState("");
  const [providerFilter, setProviderFilter] = useState("all");
  const [verifyingId, setVerifyingId] = useState<string | null>(null);
  const [verifyError, setVerifyError] = useState<string | null>(null);

  const verifyTools = async (id: string) => {
    setVerifyingId(id);
    setVerifyError(null);
    try {
      await api.verifyTools(id);
      await queryClient.invalidateQueries({ queryKey: MODELS_QUERY_KEY });
    } catch (err) {
      setVerifyError(errorMessage(err) || t("エラー"));
    } finally {
      setVerifyingId(null);
    }
  };

  const toggleAuto = async (id: string, auto: boolean) => {
    setVerifyError(null);
    try {
      await api.setModelAuto(id, auto);
      await queryClient.invalidateQueries({ queryKey: MODELS_QUERY_KEY });
    } catch (err) {
      setVerifyError(errorMessage(err) || t("エラー"));
    }
  };

  const providerLabel = (id: string) =>
    providers.data?.find((p) => p.id === id)?.label ?? id;

  const groups = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    const list = (models.data ?? []).filter((m) => {
      if (providerFilter !== "all" && m.provider_id !== providerFilter)
        return false;
      if (!needle) return true;
      return [m.label, m.id, providerLabel(m.provider_id)].some((v) =>
        v.toLocaleLowerCase().includes(needle),
      );
    });
    const byProvider = new Map<string, ModelView[]>();
    for (const model of list) {
      const bucket = byProvider.get(model.provider_id);
      if (bucket) bucket.push(model);
      else byProvider.set(model.provider_id, [model]);
    }
    return [...byProvider.entries()];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [models.data, providerFilter, providers.data, query]);

  const total = models.data?.length ?? 0;
  const autoCount = (models.data ?? []).filter((m) => m.auto_candidate).length;
  const shown = groups.reduce((sum, [, rows]) => sum + rows.length, 0);

  return (
    <Section>
      {verifyError && <ErrorBox error={verifyError} />}
      {models.isPending ? (
        <ListSkeleton rows={4} />
      ) : models.isError ? (
        <ErrorBox error={models.error} />
      ) : total === 0 ? (
        <EmptyState
          icon={<Sparkles size={22} />}
          title={t("モデルがありません")}
          sub={t("プロバイダーを追加してモデルを取得してください")}
          action={
            <Button
              variant="primary"
              size="sm"
              onClick={() => openSettings("providers")}
            >
              {t("プロバイダーを追加")}
            </Button>
          }
        />
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <div className="min-w-48 flex-1">
              <SearchField
                icon={<Search size={13} />}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onClear={query ? () => setQuery("") : undefined}
                clearLabel={t("クリア")}
                aria-label={t("モデルを検索")}
                placeholder={t("モデルを検索")}
              />
            </div>
            <div className="w-48 shrink-0">
              <Select
                value={providerFilter}
                onChange={(e) => setProviderFilter(e.target.value)}
                aria-label={t("プロバイダーで絞り込む")}
              >
                <option value="all">{t("すべてのプロバイダー")}</option>
                {(providers.data ?? []).map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.label}
                  </option>
                ))}
              </Select>
            </div>
          </div>

          <div className="text-xs text-faint">
            {tf("{shown} / {total} 件のモデル", { shown, total })}
            {" · "}
            {tf("Auto候補: {n} 件", { n: autoCount })}
          </div>

          {shown === 0 ? (
            <SearchEmptyState
              title={t("該当するモデルがありません")}
              onClear={() => {
                setQuery("");
                setProviderFilter("all");
              }}
            />
          ) : (
            <div className="flex flex-col gap-4">
              {groups.map(([providerId, rows]) => (
                <div key={providerId} className="flex flex-col gap-2">
                  <SectionLabel
                    className="px-0 pb-0"
                    trailing={
                      <Badge size="sm" className="font-mono">
                        {rows.length}
                      </Badge>
                    }
                  >
                    {providerLabel(providerId)}
                  </SectionLabel>
                  <div className="overflow-hidden rounded-xl border border-line bg-surface shadow-xs divide-y divide-line">
                    {rows.map((m) => (
                      <div
                        key={m.id}
                        className="flex flex-wrap items-center gap-x-4 gap-y-2 px-3 py-2.5 transition-colors hover:bg-hover"
                      >
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-1.5">
                            <span className="truncate text-sm font-medium text-fg">
                              {m.label}
                            </span>
                          </div>
                          <div
                            className="truncate font-mono text-2xs text-faint"
                            title={m.id}
                          >
                            {shortModelId(m.id)}
                          </div>
                          {(m.family ||
                            m.parameter_size ||
                            m.quantization ||
                            (m.size_bytes ?? 0) > 0 ||
                            m.loaded ||
                            m.capability_source === "native") && (
                            <div className="mt-1 flex flex-wrap items-center gap-1">
                              {m.loaded && (
                                <Badge tone="success" size="sm">
                                  {t("ロード中")}
                                </Badge>
                              )}
                              {m.family && <Badge size="sm">{m.family}</Badge>}
                              {m.parameter_size && (
                                <Badge size="sm">{m.parameter_size}</Badge>
                              )}
                              {m.quantization && (
                                <Badge size="sm">{m.quantization}</Badge>
                              )}
                              {(m.size_bytes ?? 0) > 0 && (
                                <span className="text-2xs text-faint">
                                  {formatBytes(m.size_bytes ?? 0)}
                                </span>
                              )}
                              {m.capability_source === "native" && (
                                <Badge size="sm">{t("ネイティブ")}</Badge>
                              )}
                            </div>
                          )}
                        </div>
                        <CapabilityRow model={m} />
                        <div className="flex items-center gap-2 text-2xs text-muted">
                          <span className="text-faint">
                            {t("コンテキスト")}
                          </span>
                          <span className="font-mono">
                            {formatContext(m.context_window)}
                          </span>
                        </div>
                        <label className="flex items-center gap-1.5 text-2xs text-muted">
                          <Zap
                            size={11}
                            className="text-faint"
                            aria-hidden="true"
                          />
                          {t("Auto")}
                          <Switch
                            checked={m.auto_candidate}
                            onCheckedChange={(checked) =>
                              void toggleAuto(m.id, checked)
                            }
                            aria-label={t("Auto候補にする")}
                          />
                        </label>
                        <Button
                          size="sm"
                          variant="ghost"
                          disabled={verifyingId === m.id}
                          onClick={() => void verifyTools(m.id)}
                        >
                          <Sparkles size={12} /> {t("能力を確認")}
                        </Button>
                      </div>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          )}
        </>
      )}

      <p className="inline-flex items-start gap-1 text-xs text-muted">
        <Info size={12} className="mt-0.5 shrink-0" aria-hidden="true" />
        {t(
          "モデルの対応機能は確認結果をもとに表示されます。誤りがあれば再確認してください。",
        )}
      </p>
    </Section>
  );
}
