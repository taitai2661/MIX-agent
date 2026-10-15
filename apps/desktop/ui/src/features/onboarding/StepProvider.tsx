import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Cloud, MonitorCog, Plug } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t, tf } from "@/lib/i18n";
import { MODELS_QUERY_KEY, PROVIDERS_QUERY_KEY, useModels, useProviderPresets, useProviders } from "@/lib/queries";
import { Field, Row } from "@/components/layout";
import { ErrorBox } from "@/components/shared";
import { Alert } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Select } from "@/components/ui/input";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";
import { Spinner } from "@/components/ui/spinner";
import { LocalRuntimeScan } from "@/features/providers/LocalRuntimeScan";

/**
 * Step 1: connect a model provider.
 *
 * Two equally-weighted tabs because neither route is right for everyone: a
 * local runtime costs nothing and never leaves the machine, while a cloud API
 * is the only option for most people who do not already run Ollama. The local
 * tab leads because it needs no credential at all — the shortest possible path
 * from "installed the app" to "it answered".
 */
export function StepProvider({ onDone }: { onDone: () => void }) {
  const [tab, setTab] = useState<"local" | "cloud">("local");

  return (
    <div className="flex flex-col gap-4">
      <Segmented aria-label={t("モデル提供元の種類")} className="w-full">
        {(
          [
            { id: "local" as const, label: t("この PC で動かす"), Icon: MonitorCog },
            { id: "cloud" as const, label: t("クラウド API"), Icon: Cloud },
          ]
        ).map((opt) => (
          <SegmentedOption
            key={opt.id}
            value={opt.id}
            label={opt.label}
            icon={<opt.Icon size={13} aria-hidden="true" />}
            selected={tab === opt.id}
            onSelect={setTab}
            data-testid={`onboarding-provider-tab-${opt.id}`}
          >
            {opt.label}
          </SegmentedOption>
        ))}
      </Segmented>

      {tab === "local"
        ? <StepProviderLocal onDone={onDone} />
        : <StepProviderCloud onDone={onDone} />}
    </div>
  );
}

/** Local tab: delegate to the shared sweep, and treat "we found and imported
 * something" as the success signal rather than asking for a second click. */
function StepProviderLocal({ onDone }: { onDone: () => void }) {
  return (
    <div className="flex flex-col gap-3">
      <p className="m-0 text-xs text-muted">
        {t("API キーは不要です。この端末の上だけで動きます。")}
      </p>
      <LocalRuntimeScan
        intro={t("LM Studio や Ollama などが起動中か調べます。")}
        emptyHint={t("見つかりませんでした。LM Studio か Ollama を起動してからもう一度検出するか、「クラウド API」タブを使ってください。")}
        onImported={onDone}
      />
    </div>
  );
}

/** Cloud tab: preset → base URL → key, then one button that both tests the
 * credential and pulls the model list. `connect_provider` is that button: it
 * fails with a readable message when the key, the URL or the account is wrong,
 * and it is the only thing that makes a freshly added provider usable. */
function StepProviderCloud({ onDone }: { onDone: () => void }) {
  const queryClient = useQueryClient();
  const presets = useProviderPresets();
  const providers = useProviders();
  const models = useModels();

  const [presetSlug, setPresetSlug] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [error, setError] = useState<string | null>(null);

  const preset = presets.data?.find((p) => p.slug === presetSlug) ?? null;

  // A provider already on file (from a previous attempt, or one the user added
  // in Settings) makes this step a pure "test + fetch". Reusing it rather than
  // creating a second one is what keeps a retry after a failed key from leaving
  // a duplicate row behind.
  //
  // Only when no preset is chosen: someone who just imported a local runtime and
  // then switched to this tab is adding a *second* provider, and overwriting the
  // first one's URL would be a nasty surprise.
  const reusable =
    !presetSlug && providers.data?.length === 1 ? providers.data[0] : null;

  const connect = useMutation({
    mutationFn: async () => {
      setError(null);
      const provider = reusable
        ? await api.updateProvider({
            id: reusable.id,
            label: null,
            base_url: baseUrl || reusable.base_url,
            api_key: apiKey || null,
            enabled: null,
          })
        : await api.createProvider({
            preset_slug: preset?.slug ?? null,
            kind: preset?.kind ?? "compatible",
            label: preset?.label ?? "Provider",
            base_url: baseUrl || (preset?.default_url ?? null),
            api_key: apiKey || null,
            enabled: true,
          });
      await api.connectProvider(provider.id);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY });
      void queryClient.invalidateQueries({ queryKey: MODELS_QUERY_KEY });
      onDone();
    },
    onError: (err) => setError(errorMessage(err)),
  });

  return (
    <div className="flex flex-col gap-3">
      <Alert tone="info">
        {t("API キーは OS の資格情報ストレージに保存され、設定画面には表示されません。")}
      </Alert>

      {!reusable ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <Field label={t("プロバイダー")}>
            <Select
              value={presetSlug}
              onChange={(e) => {
                setPresetSlug(e.target.value);
                const next = presets.data?.find((p) => p.slug === e.target.value);
                if (next) setBaseUrl(next.default_url || "");
              }}
              disabled={presets.isPending}
            >
              <option value="">
                {presets.isPending ? t("読み込み中…") : t("選択してください…")}
              </option>
              {(presets.data ?? []).map((p) => (
                <option key={p.slug} value={p.slug}>{p.label}</option>
              ))}
            </Select>
          </Field>
          <Field label="Base URL">
            <Input
              placeholder={t("プリセットを選ぶと入力されます")}
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
            />
          </Field>
          <Field
            label={t("API キー")}
            hint={
              preset?.api_key_required
                ? t("このプロバイダーでは API キーが必要です")
                : t("このプロバイダーでは API キーは任意です")
            }
          >
            <Input
              type="password"
              placeholder="sk-…"
              autoComplete="off"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
            />
          </Field>
        </div>
      ) : (
        <Row>
          <Plug size={12} className="text-muted" />
          <span className="text-xs text-fg">{reusable.label}</span>
          <code className="font-mono text-xs text-faint">{reusable.base_url ?? "—"}</code>
        </Row>
      )}

      <Row>
        <Button
          variant="primary"
          disabled={
            (!reusable && !presetSlug) ||
            connect.isPending ||
            // A preset that declares a key requirement is not going to answer
            // without one, and the resulting HTTP 401 is a much worse error to
            // read than saying so here.
            (!!preset?.api_key_required && !apiKey.trim())
          }
          onClick={() => connect.mutate()}
        >
          {connect.isPending
            ? <Spinner size={12} />
            : <Plug size={12} />}
          {connect.isPending ? t("確認中…") : t("接続してモデル一覧を取得")}
        </Button>
        {(models.data?.length ?? 0) > 0 && (
          <Badge tone="success" size="sm">
            {tf("{n} モデル", { n: models.data?.length ?? 0 })}
          </Badge>
        )}
      </Row>

      {error && <ErrorBox error={error} />}
    </div>
  );
}
