import { useMemo, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Cloud,
  KeyRound,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Server,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { Card, Field, Row, Section } from "@/components/layout";
import {
  EmptyState,
  ErrorBox,
  ListSkeleton,
  payloadBlock,
} from "@/components/shared";
import { DeleteIconButton, SearchEmptyState } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Input, SearchField, Select } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  MODELS_QUERY_KEY,
  PROVIDERS_QUERY_KEY,
  useModels,
  useProviderPresets,
  useProviders,
} from "@/lib/queries";
import { LocalDiscoveryCard } from "./LocalDiscoveryCard";
import type { ProviderPresetView, ProviderView } from "@/lib/types";

function ProviderIcon({ kind }: { kind: string }) {
  const local =
    kind === "compatible" || kind === "ollama" || kind === "lmstudio";
  const Icon = local ? Server : Cloud;
  return <Icon size={15} className="shrink-0 text-faint" aria-hidden="true" />;
}

export function ProvidersPane() {
  const queryClient = useQueryClient();
  const providers = useProviders();
  const models = useModels();
  const presets = useProviderPresets();

  const [draftPreset, setDraftPreset] = useState("");
  const [draftLabel, setDraftLabel] = useState("");
  const [draftBaseUrl, setDraftBaseUrl] = useState("");
  const [draftApiKey, setDraftApiKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editLabel, setEditLabel] = useState("");
  const [editBaseUrl, setEditBaseUrl] = useState("");
  const [editApiKey, setEditApiKey] = useState("");

  const selectedPreset =
    presets.data?.find((p) => p.slug === draftPreset) ?? null;

  const createProvider = useMutation({
    mutationFn: () =>
      api.createProvider({
        preset_slug: selectedPreset?.slug ?? null,
        kind: selectedPreset?.kind ?? "compatible",
        label: draftLabel || selectedPreset?.label || "Provider",
        base_url: draftBaseUrl || (selectedPreset?.default_url ?? null),
        api_key: draftApiKey || null,
        enabled: true,
      }),
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

  const updateProvider = useMutation({
    mutationFn: (input: {
      id: string;
      label: string;
      base_url: string | null;
      api_key: string | null;
      enabled: boolean;
    }) => api.updateProvider(input),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY });
      setEditingId(null);
      setError(null);
    },
    onError: (e) => setError(errorMessage(e)),
  });

  const toggleEnabled = (p: ProviderView) => {
    setError(null);
    updateProvider.mutate({
      id: p.id,
      label: p.label,
      base_url: p.base_url,
      api_key: null,
      enabled: !p.enabled,
    });
  };

  const startEdit = (p: ProviderView) => {
    setError(null);
    setEditingId(p.id);
    setEditLabel(p.label);
    setEditBaseUrl(p.base_url ?? "");
    setEditApiKey("");
  };

  const deleteProvider = useMutation({
    mutationFn: (id: string) => api.deleteProvider(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY });
      void queryClient.invalidateQueries({ queryKey: MODELS_QUERY_KEY });
    },
    onError: (e) => setError(errorMessage(e)),
  });

  const [syncCounts, setSyncCounts] = useState<Record<string, number>>({});
  const syncProvider = useMutation({
    mutationFn: (id: string) => api.connectProvider(id),
    onSuccess: (count, id) => {
      setSyncCounts((prev) => ({ ...prev, [id]: count }));
      void queryClient.invalidateQueries({ queryKey: MODELS_QUERY_KEY });
    },
    onError: (e) => setError(errorMessage(e)),
  });

  const modelCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const model of models.data ?? []) {
      counts.set(model.provider_id, (counts.get(model.provider_id) ?? 0) + 1);
    }
    return counts;
  }, [models.data]);

  const onPresetChange = (slug: string) => {
    setDraftPreset(slug);
    const preset = presets.data?.find((x) => x.slug === slug);
    if (preset) setDraftBaseUrl(preset.default_url || "");
  };

  // Group presets by category so a fifty-entry select is navigable.
  const presetGroups = useMemo(() => {
    const groups = new Map<string, ProviderPresetView[]>();
    for (const preset of presets.data ?? []) {
      const bucket = groups.get(preset.category);
      if (bucket) bucket.push(preset);
      else groups.set(preset.category, [preset]);
    }
    return [...groups.entries()];
  }, [presets.data]);

  // `?? []` allocates a fresh array every render while the query is in flight,
  // which would invalidate `filtered` on each one.
  const list = useMemo(() => providers.data ?? [], [providers.data]);
  const filtered = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    if (!needle) return list;
    return list.filter((p) =>
      [p.label, p.kind, p.base_url ?? "", p.preset ?? ""].some((v) =>
        v.toLocaleLowerCase().includes(needle),
      ),
    );
  }, [list, query]);

  return (
    <Section>
      <LocalDiscoveryCard />

      {error && <ErrorBox error={error} />}

      <Card title={t("プロバイダーを追加")} icon={<Plus size={13} />}>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label={t("プリセット")}>
            <Select
              value={draftPreset}
              onChange={(e) => onPresetChange(e.target.value)}
              disabled={presets.isPending}
            >
              <option value="">
                {presets.isPending ? t("読み込み中…") : t("プリセットを選択…")}
              </option>
              {presetGroups.map(([category, items]) => (
                <optgroup key={category} label={category}>
                  {items.map((p) => (
                    <option key={p.slug} value={p.slug}>
                      {p.label}
                    </option>
                  ))}
                </optgroup>
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
          <Field
            label="Base URL"
            hint={
              selectedPreset?.default_url
                ? t("プリセットの既定値を編集できます")
                : undefined
            }
          >
            <Input
              placeholder="https://api.example.com/v1"
              value={draftBaseUrl}
              onChange={(e) => setDraftBaseUrl(e.target.value)}
            />
          </Field>
          <Field
            label={t("API キー")}
            hint={
              selectedPreset
                ? selectedPreset.api_key_required
                  ? t("このプロバイダーでは API キーが必要です")
                  : t("このプロバイダーでは API キーは任意です")
                : undefined
            }
          >
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
            {createProvider.isPending ? (
              <Spinner size={12} />
            ) : (
              <Plus size={12} />
            )}
            {t("作成")}
          </Button>
        </Row>
      </Card>

      {providers.isPending ? (
        <ListSkeleton rows={3} />
      ) : providers.isError ? (
        <ErrorBox error={providers.error} />
      ) : list.length === 0 ? (
        <EmptyState
          icon={<KeyRound size={22} />}
          title={t("プロバイダーが未設定です")}
          sub={t("プリセットを選んで API キーを入力してください。")}
        />
      ) : (
        <>
          {list.length > 3 && (
            <SearchField
              icon={<Search size={13} />}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onClear={query ? () => setQuery("") : undefined}
              clearLabel={t("クリア")}
              aria-label={t("プロバイダーを検索")}
              placeholder={t("プロバイダーを検索")}
            />
          )}

          {filtered.length === 0 ? (
            <SearchEmptyState
              title={t("該当するプロバイダーがありません")}
              onClear={() => setQuery("")}
            />
          ) : (
            <div className="grid gap-3 lg:grid-cols-2">
              {filtered.map((p) => {
                const editing = editingId === p.id;
                const count = modelCounts.get(p.id) ?? 0;
                const synced = syncCounts[p.id];
                return (
                  <Card key={p.id}>
                    {editing ? (
                      <div className="flex flex-col gap-3">
                        <Field label={t("ラベル")}>
                          <Input
                            value={editLabel}
                            onChange={(e) => setEditLabel(e.target.value)}
                          />
                        </Field>
                        <Field label="Base URL">
                          <Input
                            value={editBaseUrl}
                            onChange={(e) => setEditBaseUrl(e.target.value)}
                          />
                        </Field>
                        <Field
                          label={t("API キー")}
                          hint={t("空欄のままにすると既存のキーを保持します")}
                        >
                          <Input
                            type="password"
                            placeholder="••••••••"
                            value={editApiKey}
                            onChange={(e) => setEditApiKey(e.target.value)}
                          />
                        </Field>
                        <Row align="end">
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => setEditingId(null)}
                          >
                            {t("キャンセル")}
                          </Button>
                          <Button
                            variant="primary"
                            size="sm"
                            disabled={
                              !editLabel.trim() || updateProvider.isPending
                            }
                            onClick={() =>
                              updateProvider.mutate({
                                id: p.id,
                                label: editLabel.trim(),
                                base_url: editBaseUrl.trim() || null,
                                api_key: editApiKey || null,
                                enabled: p.enabled,
                              })
                            }
                          >
                            {updateProvider.isPending ? (
                              <Spinner size={12} />
                            ) : null}
                            {t("保存")}
                          </Button>
                        </Row>
                      </div>
                    ) : (
                      <>
                        <div className="flex items-start gap-2.5">
                          <ProviderIcon kind={p.kind} />
                          <div className="min-w-0 flex-1">
                            <div className="truncate text-sm font-medium text-fg">
                              {p.label}
                            </div>
                            <div className="mt-0.5 flex flex-wrap items-center gap-1.5">
                              <Badge size="sm" className="font-mono">
                                {p.kind}
                              </Badge>
                              {p.has_api_key ? (
                                <Badge tone="success" size="sm">
                                  {t("保存済み")}
                                </Badge>
                              ) : (
                                <Badge tone="warning" size="sm">
                                  {t("未設定")}
                                </Badge>
                              )}
                              {p.preset && (
                                <span className="truncate text-2xs text-faint">
                                  {p.preset}
                                </span>
                              )}
                            </div>
                          </div>
                          <label className="flex shrink-0 items-center gap-1.5 text-2xs text-muted">
                            <span>{p.enabled ? t("有効") : t("無効")}</span>
                            <Switch
                              checked={p.enabled}
                              onCheckedChange={() => toggleEnabled(p)}
                              aria-label={tf("{name} を有効にする", {
                                name: p.label,
                              })}
                            />
                          </label>
                        </div>

                        <div
                          className={payloadBlock}
                          title={p.base_url ?? undefined}
                        >
                          {p.base_url ?? "—"}
                        </div>

                        <div className="flex flex-wrap items-center gap-2 text-xs text-faint">
                          <span>{tf("{n} モデル", { n: count })}</span>
                          {synced !== undefined && (
                            <Badge tone="success" size="sm">
                              {tf("{n} 件を取り込みました", { n: synced })}
                            </Badge>
                          )}
                        </div>

                        <Row align="between" className="gap-1.5">
                          <Button
                            variant="ghost"
                            size="sm"
                            disabled={syncProvider.isPending}
                            onClick={() => syncProvider.mutate(p.id)}
                          >
                            {syncProvider.isPending &&
                            syncProvider.variables === p.id ? (
                              <Spinner size={12} />
                            ) : (
                              <RefreshCw size={12} />
                            )}
                            {t("接続してモデルを取得")}
                          </Button>
                          <span className="flex items-center gap-1">
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              title={t("編集")}
                              aria-label={t("編集")}
                              onClick={() => startEdit(p)}
                            >
                              <Pencil size={13} />
                            </Button>
                            <DeleteIconButton
                              title={t("プロバイダーを削除しますか？")}
                              description={t(
                                "このプロバイダーのモデル一覧も消えます。",
                              )}
                              pending={deleteProvider.isPending}
                              onDelete={() => deleteProvider.mutate(p.id)}
                            />
                          </span>
                        </Row>
                      </>
                    )}
                  </Card>
                );
              })}
            </div>
          )}
        </>
      )}

      {syncProvider.isError && <ErrorBox error={syncProvider.error} />}
      {deleteProvider.isError && <ErrorBox error={deleteProvider.error} />}
    </Section>
  );
}
