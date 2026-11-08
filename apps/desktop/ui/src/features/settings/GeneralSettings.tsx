import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Brain,
  Clock3,
  Compass,
  Keyboard,
  Lock,
  ServerCog,
  Trash2,
  Zap,
} from "lucide-react";
import { api } from "@/lib/api";
import { ErrorBadge } from "@/components/shared";
import { t, tf } from "@/lib/i18n";
import { modKey } from "@/lib/platform";
import { useUi } from "@/lib/ui";
import { useOnboardingGate } from "@/features/onboarding/useOnboardingGate";
import { Card, Field, Row, Section } from "@/components/layout";
import { ToggleRow } from "@/components/common";
import { Input, Select } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import { Segmented, SegmentedOption } from "@/components/ui/segmented";
import { ModelPicker } from "@/features/chat/ModelPicker";
import { useModels, useProviders } from "@/lib/queries";
import { useSettingsForm } from "@/lib/settings";
import { clearPromptHistory, promptHistoryCount } from "@/lib/promptHistory";
import { clearComposerDrafts, countComposerDrafts } from "@/lib/drafts";
import { NetworkSettings } from "./NetworkSettings";

export function GeneralSettings() {
  const { data, patch } = useSettingsForm();
  const onboarding = useOnboardingGate();
  const models = useModels();
  const providers = useProviders();
  const sendShortcut = useUi((s) => s.sendShortcut);
  const setSendShortcut = useUi((s) => s.setSendShortcut);
  const { confirm, dialog } = useConfirm();
  const monthlySpend = useQuery({
    queryKey: ["monthly-spend"],
    queryFn: api.getMonthlySpend,
    refetchInterval: 60_000,
  });
  const [historyCount, setHistoryCount] = useState(() => promptHistoryCount());
  const [draftCount, setDraftCount] = useState(() => countComposerDrafts());
  const queryClient = useQueryClient();
  // The OS is the source of truth for autostart (a user may have deleted the
  // launch agent by hand), so read it back instead of storing a second copy in
  // app_settings that could disagree.
  const autostart = useQuery({
    queryKey: ["autostart"],
    queryFn: api.getAutostart,
    staleTime: Infinity,
  });
  const sandbox = useQuery({
    queryKey: ["sandbox_status"],
    queryFn: api.getSandboxStatus,
    staleTime: Infinity,
  });
  const autostartMutation = useMutation({
    mutationFn: api.setAutostart,
    onSuccess: (value) => queryClient.setQueryData(["autostart"], value),
  });
  const modifierSendLabel = tf("{shortcut}+Enter で送信 (Enter で改行)", {
    shortcut: modKey(),
  });

  const clearHistory = async () => {
    if (historyCount === 0) return;
    if (
      await confirm({
        title: t("プロンプト履歴を消去しますか？"),
        description: t(
          "入力欄で矢印キーを押したときに出る、過去の入力履歴を削除します。会話そのものは削除されません。",
        ),
        confirmLabel: t("消去"),
        tone: "danger",
      })
    ) {
      clearPromptHistory();
      setHistoryCount(0);
    }
  };

  const clearDrafts = async () => {
    if (draftCount === 0) return;
    if (
      await confirm({
        title: t("下書きを消去しますか？"),
        description: t(
          "まだ送信していない入力内容を、すべてのスレッドから削除します。元に戻せません。",
        ),
        confirmLabel: t("消去"),
        tone: "danger",
      })
    ) {
      clearComposerDrafts();
      setDraftCount(0);
    }
  };

  return (
    <Section>
      <Card title={t("セットアップガイド")} icon={<Compass size={13} />}>
        <Row align="between">
          <p className="m-0 max-w-md text-xs text-muted">
            {t(
              "モデル提供元の追加方法と、作業フォルダの選び方を確認できます。",
            )}
          </p>
          <Button size="sm" onClick={onboarding.reopen}>
            <Compass size={12} /> {t("もう一度見る")}
          </Button>
        </Row>
      </Card>

      <Card title={t("初期設定")} icon={<Zap size={13} />}>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field
            label={t("モデル")}
            hint={t("新しいスレッドで既定で使うモデル")}
          >
            <ModelPicker
              models={models.data ?? []}
              value={data.default_model_id ?? ""}
              onChange={(id) => patch({ default_model_id: id })}
              providers={providers.data ?? []}
              pending={models.isPending}
              className="w-full max-w-none"
            />
          </Field>
          <Field label={t("モード")}>
            <Select
              value={data.default_mode ?? "chat"}
              onChange={(e) => patch({ default_mode: e.target.value })}
            >
              <option value="chat">{t("チャット")}</option>
              <option value="thinking">{t("思考")}</option>
              <option value="agent">{t("自動実行")}</option>
            </Select>
          </Field>
        </div>
      </Card>

      <Card title={t("検証と整形の既定値")} icon={<ServerCog size={13} />}>
        <div className="grid gap-3 sm:grid-cols-2">
          <Field
            label={t("検証コマンド")}
            hint={t(
              "プロジェクト固有の指定がないときに使う確認コマンドです。空欄なら検証ゲートは無効です。",
            )}
          >
            <Input
              value={data.verify_command ?? ""}
              placeholder="npm test"
              onChange={(e) =>
                patch({ verify_command: e.target.value || null })
              }
            />
          </Field>
          <Field
            label={t("整形コマンド")}
            hint={t("編集後に自動実行する整形・lintコマンドです。")}
          >
            <Input
              value={data.format_command ?? ""}
              placeholder="cargo fmt"
              onChange={(e) =>
                patch({ format_command: e.target.value || null })
              }
            />
          </Field>
        </div>
      </Card>

      <Card title={t("バックグラウンド実行")} icon={<Clock3 size={13} />}>
        <ToggleRow
          label={t("ログイン時に起動")}
          hint={t(
            "OS のログイン時に MIX-agent をトレイ常駐で起動します。スケジュールをアプリ未起動のまま実行するために必要です。",
          )}
          checked={autostart.data ?? false}
          disabled={autostart.isPending || autostartMutation.isPending}
          onChange={(next) => autostartMutation.mutate(next)}
        />
        {autostartMutation.isError && (
          <ErrorBadge error={autostartMutation.error} />
        )}
        <p className="m-0 text-xs text-muted">
          {t(
            "ウィンドウを閉じてもトレイに残り、スケジュールと実行中のタスクは継続します。完全に終了するにはトレイメニューの「Quit」を使います。",
          )}
        </p>
        <Field
          label={t("承認待ちの保持時間（分）")}
          hint={t(
            "エージェントが承認を待つ時間です。この時間を過ぎると自動的に拒否されます。離席することが多い場合は長めにしてください（1〜10080分）。",
          )}
        >
          <Input
            type="number"
            min={1}
            max={10080}
            value={data.approval_ttl_minutes}
            onChange={(e) =>
              patch({
                approval_ttl_minutes: Math.min(
                  10080,
                  Math.max(1, Number(e.target.value) || 1),
                ),
              })
            }
          />
        </Field>
      </Card>

      <Card title={t("ターミナルのサンドボックス")} icon={<Lock size={13} />}>
        <ToggleRow
          label={t("シェルをサンドボックス内で実行")}
          hint={
            sandbox.data?.platform === "linux"
              ? t(
                  "bubblewrap (bwrap) を使い、ワークスペースとビルドキャッシュ以外への書き込みを拒否します。次に起動するシェルから適用されます。",
                )
              : sandbox.data?.platform === "windows"
                ? t(
                    "Job Object でシェルとその子プロセスを束ね、プロセス数の上限とアプリ終了時の一括停止を強制します。ファイルシステムの書き込み隔離はOSレベルでは未対応です。",
                  )
                : t(
                    "macOS では sandbox-exec を使い、ワークスペースと一時ディレクトリ以外への書き込みを拒否します。次に起動するシェルから適用されます。",
                  )
          }
          checked={data.sandbox_terminal}
          onChange={(next) =>
            patch({
              sandbox_terminal: next,
              // The network profile only means anything with the sandbox on;
              // turning the parent off clears it in the same save.
              ...(next ? {} : { sandbox_terminal_network: false }),
            })
          }
          disabled={!sandbox.data?.available}
        />
        <ToggleRow
          label={t("シェルからのネットワーク通信も拒否")}
          hint={t(
            "パッケージ導入やテストが通らなくなるため既定はOFFです。信頼できないコードを動かすときだけ有効にしてください。",
          )}
          checked={data.sandbox_terminal_network}
          onChange={(next) => patch({ sandbox_terminal_network: next })}
          disabled={
            !sandbox.data?.available ||
            !sandbox.data?.network_supported ||
            !data.sandbox_terminal
          }
        />
        {sandbox.data && !sandbox.data.available && (
          <p className="m-0 text-xs text-danger">
            {sandbox.data.reason ??
              t("この環境ではサンドボックスを利用できません。")}
          </p>
        )}
      </Card>

      <Card title={t("コスト管理")} icon={<Zap size={13} />}>
        <Field
          label={t("月次予算 (USD)")}
          hint={t(
            "この額に達するとクラウドモデルの新しい実行を開始しません（0 または空欄で無効）。ローカルモデルは対象外です。",
          )}
        >
          <Input
            type="number"
            min={0}
            step={0.01}
            placeholder={t("無制限")}
            value={data.monthly_budget_usd ?? ""}
            onChange={(e) =>
              patch({
                monthly_budget_usd:
                  e.target.value === "" ? null : Number(e.target.value),
              })
            }
          />
        </Field>
        <p className="m-0 text-xs text-muted">
          {tf("今月の推定使用額: ${amount}", {
            amount: (monthlySpend.data ?? 0).toFixed(2),
          })}
        </p>
      </Card>

      <Card title={t("メッセージ送信キー")} icon={<Keyboard size={13} />}>
        <div className="flex flex-col gap-2">
          <p className="m-0 text-xs text-muted">
            {t("チャット入力欄でのメッセージ送信ショートカットを設定します。")}
          </p>
          <Segmented aria-label={t("メッセージ送信キー")} className="w-full">
            <SegmentedOption
              value="enter"
              label={t("Enter で送信 (Shift+Enter で改行)")}
              selected={sendShortcut === "enter"}
              onSelect={() => setSendShortcut("enter")}
            >
              {t("Enter で送信 (Shift+Enter で改行)")}
            </SegmentedOption>
            <SegmentedOption
              value="modifier"
              label={modifierSendLabel}
              selected={sendShortcut === "modifier"}
              onSelect={() => setSendShortcut("modifier")}
            >
              {modifierSendLabel}
            </SegmentedOption>
          </Segmented>
        </div>
      </Card>

      <Card title={t("Auto ルーティング")} icon={<Compass size={13} />}>
        <Field
          label={t("優先度")}
          hint={t(
            "Auto がモデルを選ぶときの基準です。速度は実測 TTFT と tokens/s が貯まってから効きます。",
          )}
        >
          <Select
            value={data.auto_priority}
            onChange={(e) =>
              patch({
                auto_priority: e.target.value as
                  | "balanced"
                  | "speed"
                  | "quality"
                  | "cost",
              })
            }
          >
            <option value="balanced">{t("バランス（推奨）")}</option>
            <option value="speed">{t("速度を優先")}</option>
            <option value="quality">{t("品質を優先")}</option>
            <option value="cost">{t("コストを優先")}</option>
          </Select>
        </Field>
        <Field
          label={t("フォールバック")}
          hint={t(
            "最初の応答前に失敗した場合、別の Auto 候補で再試行します。応答が始まった後は再試行しません。",
          )}
        >
          <Select
            value={String(data.auto_fallback_attempts)}
            onChange={(e) =>
              patch({
                auto_fallback_attempts: Number(e.target.value) as 0 | 1 | 2,
              })
            }
          >
            <option value="0">{t("なし")}</option>
            <option value="1">{t("1回（推奨）")}</option>
            <option value="2">{t("2回")}</option>
          </Select>
        </Field>
      </Card>

      <Card title={t("ローカルランタイム")} icon={<ServerCog size={13} />}>
        <ToggleRow
          label={t("起動時にローカルランタイムを検出")}
          hint={t(
            "アプリ起動後にループバックを走査し、設定済みの Ollama / LM Studio を見つけます。",
          )}
          checked={data.local_scan_on_start}
          onChange={(local_scan_on_start) => patch({ local_scan_on_start })}
        />
        <ToggleRow
          label={t("起動時にローカルランタイムを自動起動")}
          hint={t(
            "設定済みのローカルプロバイダーが停止していれば、アプリから起動します。プロバイダー単位で無効化できます。",
          )}
          checked={data.local_auto_start}
          onChange={(local_auto_start) => patch({ local_auto_start })}
        />
        <ToggleRow
          label={t("終了時にアプリ起動のランタイムを停止")}
          hint={t(
            "外部で起動したランタイムは停止しません。バックグラウンドで維持したい場合はオフにしてください。",
          )}
          checked={data.local_stop_on_exit}
          onChange={(local_stop_on_exit) => patch({ local_stop_on_exit })}
        />
        <ToggleRow
          label={t("Auto でローカルモデルを優先")}
          hint={t(
            "能力・コンテキスト条件を満たすローカルモデルにスコア上の小さなボーナスを与えます。",
          )}
          checked={data.local_prefer}
          onChange={(local_prefer) => patch({ local_prefer })}
        />
        <Field
          label={t("Ollama の keep_alive")}
          hint={t(
            "Ollama に送る keep_alive（例: 5m, 30m, -1）。ロード時とローカルチャットの両方に使います。",
          )}
        >
          <Input
            placeholder="5m"
            value={data.local_keep_alive ?? ""}
            onChange={(event) =>
              patch({ local_keep_alive: event.target.value || null })
            }
          />
        </Field>
        <Field
          label={t("ローカルモデルのコンテキスト上限")}
          hint={t(
            "ローカルモデルに渡す num_ctx の上限です。モデルの公称最大値（128K など）はそのまま使いません。",
          )}
        >
          <Select
            value={data.local_context_window ?? ""}
            onChange={(event) =>
              patch({
                local_context_window: event.target.value
                  ? Number(event.target.value)
                  : null,
              })
            }
          >
            <option value="">{t("自動（8K）")}</option>
            <option value="4096">4K</option>
            <option value="8192">8K</option>
            <option value="16384">16K</option>
            <option value="32768">32K</option>
            <option value="65536">64K</option>
            <option value="131072">128K</option>
          </Select>
        </Field>
      </Card>

      <Card title={t("メモリ")} icon={<Brain size={13} />}>
        <ToggleRow
          label={t("会話から自動でメモリを作成")}
          hint={t(
            "応答のあとに、事実・好み・制約だけを抽出して保存します。「覚えて」「忘れて」の指示は常に即時処理されます。",
          )}
          checked={data.memory_auto_formation}
          onChange={(memory_auto_formation) => patch({ memory_auto_formation })}
        />
        <ToggleRow
          label={t("類似するメモリ同士を自動でリンクする")}
          hint={t(
            "抽出後に埋め込みの近傍を調べ、関連する Memory をリンクします。リンクは想起のたびに1ホップ展開されます。",
          )}
          checked={data.memory_link_embedding_neighbors}
          onChange={(memory_link_embedding_neighbors) =>
            patch({ memory_link_embedding_neighbors })
          }
        />
        <p className="m-0 text-xs text-muted">{t("既定はオフです。")}</p>
      </Card>

      <Card title={t("ローカルデータ")} icon={<Trash2 size={13} />}>
        <p className="m-0 text-xs text-muted">
          {t(
            "入力履歴と下書きは端末の localStorage に保存されます。下書きは30日で期限切れになり、一時モードの送信は履歴に残りません。",
          )}
        </p>
        <Row>
          <Button
            size="sm"
            onClick={() => void clearHistory()}
            disabled={historyCount === 0}
          >
            <Trash2 size={12} />{" "}
            {tf("プロンプト履歴を消去 ({count})", { count: historyCount })}
          </Button>
          <Button
            size="sm"
            onClick={() => void clearDrafts()}
            disabled={draftCount === 0}
          >
            <Trash2 size={12} />{" "}
            {tf("下書きを消去 ({count})", { count: draftCount })}
          </Button>
        </Row>
      </Card>

      <Card title={t("データベースの暗号化")} icon={<Lock size={13} />}>
        <ToggleRow
          label={t("保存時の暗号化")}
          hint={t(
            "このビルドでは未対応です。データベースは平文SQLiteのため、OSのディスク暗号化（FileVault / BitLocker / LUKS）で保護してください。APIキーなどの秘密情報はマスター鍵（OSのKeychain）で暗号化してローカルに保存されます。",
          )}
          checked={false}
          disabled
          onChange={() => {}}
        />
        <p className="m-0 text-xs text-muted">
          {t("SQLCipher対応は今後のバージョンで提供予定です。")}
        </p>
      </Card>

      {/* Network lived in its own low-frequency tab; it belongs beside the
          other runtime/app safety switches rather than as a peer of "一般". */}
      <NetworkSettings />
      {dialog}
    </Section>
  );
}
