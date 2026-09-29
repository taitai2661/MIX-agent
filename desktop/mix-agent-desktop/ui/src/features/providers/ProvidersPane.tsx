import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { KeyRound, Plus, RefreshCw, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { Card, DataTable, Field, Row, Section, td, tdStrong } from "@/components/layout";
import { ErrorBox, EmptyState } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { LocalDiscoveryCard } from "./LocalDiscoveryCard";
import type { ProviderPresetView, ProviderView } from "@/lib/types";

export function ProvidersPane() {
  const queryClient = useQueryClient();
  const providers = useQuery({ queryKey: ["providers"], queryFn: api.listProviders });
  const presets = useQuery({ queryKey: ["provider_presets"], queryFn: api.providerPresets });
  const [draftPreset, setDraftPreset] = useState("");
  const [draftLabel, setDraftLabel] = useState("");
  const [draftBaseUrl, setDraftBaseUrl] = useState("");
  const [draftApiKey, setDraftApiKey] = useState("");
  const [error, setError] = useState<string | null>(null);

  const createProvider = useMutation({
    mutationFn: () => {
      const preset = presets.data?.find((p) => p.slug === draftPreset) ?? null;
      return api.createProvider({
        preset_slug: preset?.slug ?? null,
        kind: preset?.kind ?? "compatible",
        label: draftLabel || preset?.label || "Provider",
        base_url: draftBaseUrl || (preset?.default_url ?? null),
        api_key: draftApiKey || null,
        enabled: true,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
      void queryClient.invalidateQueries({ queryKey: ["models"] });
      setDraftLabel("");
      setDraftBaseUrl("");
      setDraftApiKey("");
      setDraftPreset("");
      setError(null);
    },
    onError: (e) => setError(errorMessage(e)),
  });

  const deleteProvider = useMutation({
    mutationFn: (id: string) => api.deleteProvider(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["providers"] });
      void queryClient.invalidateQueries({ queryKey: ["models"] });
    },
  });

  // Pull the provider's model list over the network. This is also the
  // connection test: it fails loudly when the key, the URL, or the server is
  // wrong, and it is the only way a newly added provider gets models.
  const [syncCounts, setSyncCounts] = useState<Record<string, number>>({});
  const syncProvider = useMutation({
    mutationFn: (id: string) => api.connectProvider(id),
    onSuccess: (count, id) => {
      setSyncCounts((prev) => ({ ...prev, [id]: count }));
      void queryClient.invalidateQueries({ queryKey: ["models"] });
    },
  });

  const onPresetChange = (slug: string) => {
    setDraftPreset(slug);
    const preset = presets.data?.find((x) => x.slug === slug);
    if (preset) setDraftBaseUrl(preset.default_url || "");
  };

  const list = providers.data ?? [];

  return (
    <Section>
      <LocalDiscoveryCard />
      <Card title={t("プロバイダーを追加")} icon={<Plus size={13} />}>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label="Preset">
            <Select value={draftPreset} onChange={(e) => onPresetChange(e.target.value)}>
              <option value="">プリセットを選択…</option>
              {(presets.data ?? []).map((p: ProviderPresetView) => (
                <option key={p.slug} value={p.slug}>{p.label} — {p.category}</option>
              ))}
            </Select>
          </Field>
          <Field label="Label">
            <Input
              placeholder="表示名"
              value={draftLabel}
              onChange={(e) => setDraftLabel(e.target.value)}
            />
          </Field>
          <Field label="Base URL">
            <Input
              placeholder="https://api.example.com/v1"
              value={draftBaseUrl}
              onChange={(e) => setDraftBaseUrl(e.target.value)}
            />
          </Field>
          <Field label="API Key">
            <Input
              type="password"
              placeholder="sk-…"
              value={draftApiKey}
              onChange={(e) => setDraftApiKey(e.target.value)}
            />
          </Field>
        </div>
        <Row>
          <Button
            variant="primary"
            disabled={!draftPreset || createProvider.isPending}
            onClick={() => createProvider.mutate()}
          >
            <Plus size={12} /> {t("作成")}
          </Button>
          {error && <Badge tone="danger">{error}</Badge>}
        </Row>
      </Card>

      {list.length === 0 ? (
        <EmptyState
          icon={<KeyRound size={22} />}
          title={t("プロバイダーが未設定です")}
          sub="プリセットを選んで API キーを入力してください。"
        />
      ) : (
        <DataTable
          columns={["ラベル", "種類", "Base URL", "API Key", "有効", "モデル", ""]}
          rows={list}
          rowKey={(p: ProviderView) => p.id}
        >
          {(p) => (
            <>
              <td className={tdStrong}>{p.label}</td>
              <td className={td}><Badge size="sm" className="font-mono">{p.kind}</Badge></td>
              <td className={`${td} font-mono`}>{p.base_url ?? "—"}</td>
              <td className={td}>
                {p.has_api_key
                  ? <Badge tone="success" size="sm">保存済</Badge>
                  : <Badge tone="warning" size="sm">未設定</Badge>}
              </td>
              <td className={td}>
                {p.enabled
                  ? <Badge tone="success" size="sm">✓</Badge>
                  : <Badge size="sm">—</Badge>}
              </td>
              <td className={td}>
                {syncCounts[p.id] !== undefined ? (
                  <Badge tone="success" size="sm">{syncCounts[p.id]}</Badge>
                ) : (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    title={t("モデル一覧を取得")}
                    aria-label={t("モデル一覧を取得")}
                    disabled={syncProvider.isPending}
                    onClick={() => syncProvider.mutate(p.id)}
                  >
                    <RefreshCw
                      size={13}
                      className={syncProvider.isPending && syncProvider.variables === p.id
                        ? "animate-spin"
                        : undefined}
                    />
                  </Button>
                )}
              </td>
              <td className={`${td} w-12`}>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="text-danger hover:text-danger"
                  title={t("削除")}
                  aria-label={t("削除")}
                  onClick={() => {
                    // Every other destructive action in the app confirms
                    // first; losing a provider also loses its model list.
                    if (window.confirm(t("このプロバイダーを削除しますか？"))) {
                      deleteProvider.mutate(p.id);
                    }
                  }}
                >
                  <Trash2 size={13} />
                </Button>
              </td>
            </>
          )}
        </DataTable>
      )}
      {syncProvider.isError && <ErrorBox error={syncProvider.error} />}
      {deleteProvider.isError && <ErrorBox error={deleteProvider.error} />}
    </Section>
  );
}
