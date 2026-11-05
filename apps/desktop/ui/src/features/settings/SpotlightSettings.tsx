import { useMutation, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, RotateCw, Search, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { t } from "@/lib/i18n";
import { useSettingsForm } from "@/lib/settings";
import { SPOTLIGHT_STATUS_QUERY_KEY, useSpotlightStatus } from "@/lib/queries";
import { Card, Row, Section } from "@/components/layout";
import { ToggleRow } from "@/components/common";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ErrorAlert } from "@/components/shared";
import { useConfirm } from "@/components/ui/dialog";

function formatDate(value: string | null): string {
  if (!value) return t("未実行");
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

export function SpotlightSettings() {
  const { data, patch, loaded } = useSettingsForm();
  const status = useSpotlightStatus();
  const queryClient = useQueryClient();
  const { confirm, dialog } = useConfirm();
  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: SPOTLIGHT_STATUS_QUERY_KEY });

  const rebuild = useMutation({
    mutationFn: api.spotlightRebuild,
    onSuccess: invalidate,
  });
  const clear = useMutation({
    mutationFn: api.spotlightClear,
    onSuccess: invalidate,
  });

  const supported = status.data?.supported ?? true;
  const persistedEnabled = status.data?.enabled ?? false;
  const reason = status.data?.reason;

  const clearIndex = async () => {
    if (
      await confirm({
        title: t("Spotlightの索引を削除しますか？"),
        description: t(
          "MIX-agentが登録した索引をこのMacのSpotlightから削除します。設定を有効にしたままにすると、次回の変更時に再登録されます。",
        ),
        confirmLabel: t("削除"),
        tone: "danger",
      })
    ) {
      clear.mutate();
    }
  };

  return (
    <Section>
      {status.isError && <ErrorAlert error={status.error} />}
      <Card title={t("macOS Spotlight")} icon={<Search size={13} />}>
        <ToggleRow
          label={t("Spotlightに索引を提供")}
          hint={t(
            "会話・プロジェクト・ナレッジのタイトルをmacOSのSpotlight検索結果に表示します。",
          )}
          checked={data.spotlight_enabled}
          onChange={(value) => patch({ spotlight_enabled: value })}
          disabled={!loaded || (!supported && !data.spotlight_enabled)}
        />
        <ToggleRow
          label={t("本文も索引に含める")}
          hint={t(
            "会話とナレッジの本文も検索対象にします。ツール出力・添付ファイル・認証情報は含まれません。",
          )}
          checked={data.spotlight_index_body}
          onChange={(value) => patch({ spotlight_index_body: value })}
          disabled={!loaded || !data.spotlight_enabled}
        />
        <p className="m-0 text-xs text-muted">
          {t(
            "索引はこのMacのローカルにのみ保存されます。アプリを削除しても索引が残る場合があります。",
          )}
        </p>
        {!supported && (
          <div className="flex items-start gap-2 rounded-lg border border-warning/30 bg-warning-soft px-3 py-2 text-xs text-fg">
            <AlertTriangle size={14} className="mt-0.5 shrink-0" />
            <div className="flex flex-col gap-0.5">
              <span className="font-medium">
                {t("この環境ではSpotlight索引を利用できません")}
              </span>
              {reason && <span className="text-muted">{reason}</span>}
            </div>
          </div>
        )}
        {(rebuild.isError || clear.isError) && (
          <p className="m-0 text-xs text-danger">
            {t(
              "索引の操作に失敗しました。しばらくしてから再試行してください。",
            )}
          </p>
        )}
        <Row align="between">
          <span className="text-xs text-muted">
            {t("索引の再構築はバックグラウンドで実行されます。")}
          </span>
          <Row>
            <Button
              size="sm"
              disabled={!supported || !persistedEnabled || rebuild.isPending}
              onClick={() => rebuild.mutate()}
            >
              <RotateCw size={12} /> {t("今すぐ再構築")}
            </Button>
            <Button
              size="sm"
              disabled={!supported || clear.isPending}
              onClick={() => void clearIndex()}
            >
              <Trash2 size={12} /> {t("索引から削除")}
            </Button>
          </Row>
        </Row>
      </Card>

      <Card title={t("索引の状態")} icon={<RotateCw size={13} />}>
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="flex items-center justify-between text-sm">
            <span className="text-muted">{t("索引済み")}</span>
            <Badge tone="neutral">{status.data?.indexed_count ?? 0}</Badge>
          </div>
          <div className="flex items-center justify-between text-sm">
            <span className="text-muted">{t("未処理")}</span>
            <Badge tone="neutral">{status.data?.pending_count ?? 0}</Badge>
          </div>
          <div className="flex items-center justify-between text-sm">
            <span className="text-muted">{t("最終同期")}</span>
            <span className="text-xs text-fg">
              {formatDate(status.data?.last_sync ?? null)}
            </span>
          </div>
          <div className="flex items-center justify-between text-sm">
            <span className="text-muted">{t("本文索引")}</span>
            <Badge tone={status.data?.body_indexed ? "success" : "neutral"}>
              {status.data?.body_indexed ? t("有効") : t("無効")}
            </Badge>
          </div>
        </div>
        {status.data?.last_error && (
          <p className="m-0 break-words rounded-lg bg-inset px-3 py-2 font-mono text-2xs text-danger">
            {status.data.last_error}
          </p>
        )}
      </Card>
      {dialog}
    </Section>
  );
}
