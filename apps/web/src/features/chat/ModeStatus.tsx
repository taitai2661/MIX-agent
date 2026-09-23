import type { Row } from "@/app/api";

export function ModeStatus({
  mode,
  model,
  agent,
  budget: runBudget,
}: {
  mode: string;
  model?: Row;
  agent?: Row;
  budget?: { max_seconds?: number | null; max_steps?: number | null; max_tool_calls?: number | null };
}) {
  const caps = { ...model?.data.capabilities, ...model?.data.overrides };
  const canThink = caps.reasoning === true && !!model?.data.reasoning_control;
  const probe = model?.data.tool_probe;
  const toolsSupported =
    caps.tools === true || (caps.tools == null && probe?.status === "supported");
  const noTools =
    Array.isArray(agent?.data.tool_ids) && agent.data.tool_ids.length === 0;
  const policy = mode === "chat"
    ? { seconds: 1200, steps: 12, calls: 12 }
    : mode === "thinking"
      ? { seconds: 2700, steps: 24, calls: 24 }
      : { seconds: 5400, steps: 300, calls: 750 };
  const agentData = agent?.data as { max_seconds?: number; max_steps?: number; max_tool_calls?: number } | undefined;
  const seconds = runBudget?.max_seconds ?? (mode === "agent" ? agentData?.max_seconds ?? policy.seconds : policy.seconds);
  const steps = runBudget?.max_steps ?? (mode === "agent" ? agentData?.max_steps ?? policy.steps : policy.steps);
  const calls = runBudget?.max_tool_calls ?? (mode === "agent" ? agentData?.max_tool_calls ?? policy.calls : policy.calls);
  const budget = `${seconds / 60}分・${steps}ステップ・${calls} Tool Call`;
  return (
    <p className="mode-status" role="status">
      {mode === "chat"
        ? "会話の文脈に合わせて答え、必要な調査や小さな作業を進めます。"
        : mode === "thinking"
          ? "前提・別案・見落としを深く検討し、重要な結論を確認してから答えます。"
          : "計画・実行・検証・修正を繰り返し、成果を確認できた場合に完了します。"}
      <span>実行予算: {budget}。選択したモードは実行中に変わりません。</span>
      {mode !== "agent" && <span>計画・バックグラウンド処理・再開・自動Skill学習は agent 専用です。</span>}
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
