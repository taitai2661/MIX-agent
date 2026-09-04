import type { Row } from "@/app/api";

export function ModeStatus({
  mode,
  model,
  agent,
}: {
  mode: string;
  model?: Row;
  agent?: Row;
}) {
  const caps = { ...model?.data.capabilities, ...model?.data.overrides };
  const canThink = caps.reasoning === true && !!model?.data.reasoning_control;
  const probe = model?.data.tool_probe;
  const toolsSupported =
    caps.tools === true || (caps.tools == null && probe?.status === "supported");
  const noTools =
    Array.isArray(agent?.data.tool_ids) && agent.data.tool_ids.length === 0;
  const budget = mode === "chat"
    ? "15分・8ステップ・8 Tool Call"
    : mode === "thinking"
      ? "30分・16ステップ・16 Tool Call"
      : "既定60分・200ステップ・500 Tool Call";
  return (
    <p className="mode-status" role="status">
      {mode === "chat"
        ? "すばやく直接答え、必要な場合だけ思考・検索・作成・実行を使います。"
        : mode === "thinking"
          ? "前提・別案・見落としを深く検討し、重要な結論を確認してから答えます。"
          : "計画・実行・途中検証・修正を繰り返し、完了または本当の停止条件まで進めます。"}
      <span>実行予算: {budget}。選択したモードは実行中に変わりません。</span>
      {mode !== "agent" && <span>計画・委任・バックグラウンド処理・再開・自動Skill学習は agent 専用です。</span>}
      {model && mode !== "agent" && !canThink && (
        <span>
          {mode === "thinking"
            ? "通常推論で thinking を実行します。Provider固有の思考設定は送信しません。"
            : "思考制御は未対応・未確認のため、思考設定を送信しません。"}
        </span>
      )}
      {model &&
        mode !== "agent" &&
        (noTools ? (
          <span>このプリセットではツールを使いません。</span>
        ) : (
          !toolsSupported && (
            <span>
              {mode === "thinking" && probe?.status === "unknown"
                ? "Tool Callingは未確認です。初回に安全な互換性確認を行い、確認できるまではツールなしで実行します。"
                : caps.tools === false || probe?.status === "unsupported"
                  ? "このモデルはTool Calling非対応のため、ツールなしで実行します。"
                : "検索・作成・実行にはモデルのTool Calling対応確認が必要です。"}
            </span>
          )
        ))}
    </p>
  );
}

export function ReasoningSummary({ text }: { text: string }) {
  if (!text) return null;
  return (
    <details className="reasoning">
      <summary>思考の要約（Provider提供）</summary>
      <p>{text}</p>
    </details>
  );
}
