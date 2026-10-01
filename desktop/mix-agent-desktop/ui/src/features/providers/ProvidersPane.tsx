import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { KeyRound, Plus, RefreshCw, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { Card, DataTable, Field, Row, Section, td, tdStrong } from "@/components/layout";
import { ErrorBox, EmptyState, TableSkeleton } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";
import { YesNo } from "@/components/ui/status";
import { Input, Select } from "@/components/ui/input";
import { MODELS_QUERY_KEY, PROVIDERS_QUERY_KEY, useProviderPresets, useProviders } from "@/lib/queries";
import { LocalDiscoveryCard } from "./LocalDiscoveryCard";
import type { ProviderPresetView, ProviderView } from "@/lib/types";

export function ProvidersPane() {
  const queryClient = useQueryClient();
  const providers = useProviders();
  const presets = useProviderPresets();
  const [draftPreset, setDraftPreset] = useState("");
  const [draftLabel, setDraftLabel] = useState("");
  const [draftBaseUrl, setDraftBaseUrl] = useState("");
  const [draftApiKey, setDraftApiKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const { confirm, dialog } = useConfirm();

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
      void queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY });
      void queryClient.invalidateQueries({ queryKey: MODELS_QUERY_KEY });
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
      void queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY });
      void queryClient.invalidateQueries({ queryKey: MODELS_QUERY_KEY });
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
      void queryClient.invalidateQueries({ queryKey: MODELS_QUERY_KEY });
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
          <Field label={t("プリセット")}>
            <Select
              value={draftPreset}
              onChange={(e) => onPresetChange(e.target.value)}
              disabled={presets.isPending}
            >
              {/* The placeholder used to read "choose a preset" and then pop into
                  fifty real options, which looked like a mis-selection. */}
              <option value="">
                {presets.isPending ? t("読み込み中…") : t("プリセットを選択…")}
              </option>
              {(presets.data ?? []).map((p: ProviderPresetView) => (
                <option key={p.slug} value={p.slug}>{p.label} — {p.category}</option>
              ))}
            </Select>
          </Field>
          <Field label={t("ラベル")}>
            <Input
              placeholder={t("表示名")}
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
          <Field label={t("API キー")}>
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

      {providers.isPending ? (
        <TableSkeleton />
      ) : list.length === 0 ? (
        <EmptyState
          icon={<KeyRound size={22} />}
          title={t("プロバイダーが未設定です")}
          sub={t("プリセットを選んで API キーを入力してください。")}
        />
      ) : (
        <DataTable
          columns={[
            t("ラベル"), t("種類"), "Base URL", t("API キー"), t("有効"), t("モデル"), "",
          ]}
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
                  ? <Badge tone="success" size="sm">{t("保存済み")}</Badge>
                  : <Badge tone="warning" size="sm">{t("未設定")}</Badge>}
              </td>
              <td className={td}>
                <YesNo value={p.enabled} onLabel={t("有効")} offLabel={t("無効")} />
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
                    {syncProvider.isPending && syncProvider.variables === p.id
                      ? <Spinner size={13} />
                      : <RefreshCw size={13} />}
                  </Button>
                )}
              </td>
              <td className={`${td} w-12`}>
                <Button
                  variant="ghost-danger"
                  size="icon-sm"
                  title={t("削除")}
                  aria-label={t("削除")}
                  onClick={async () => {
                    if (await confirm({
                      title: t("プロバイダーを削除しますか？"),
                      description: t("このプロバイダーのモデル一覧も消えます。"),
                      confirmLabel: t("削除"),
                      tone: "danger",
                    })) {
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
      {dialog}
    </Section>
  );
}
