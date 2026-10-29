import { useState } from "react";
import { Button } from "@/components/button";
import {
  X,
  MessageSquare,
  Bot,
  Folder,
  BookOpen,
  Clock3,
  Settings2,
} from "lucide-react";
import { t } from "@/app/i18n";

const steps = [
  {
    icon: MessageSquare,
    title: "チャット",
    description:
      "AIとの対話を開始します。モデル、モード、プロジェクトを選択して、コードの作成や調査を依頼できます。",
  },
  {
    icon: Bot,
    title: "アシスタント設定",
    description:
      "用途別のAIアシスタントを作成・管理できます。モデル、モード、ツールの組み合わせを保存して使い分けられます。",
  },
  {
    icon: Folder,
    title: "プロジェクト",
    description:
      "会話、指示、資料テキストをまとめて管理します。プロジェクトごとに異なる文脈でAIを活用できます。",
  },
  {
    icon: BookOpen,
    title: "Skills",
    description:
      "検証済みの手順を保存して再利用できます。SKILL.md形式でインポート・エクスポートも可能です。",
  },
  {
    icon: Clock3,
    title: "定期実行",
    description:
      "Cron式でAgentと会話を自動実行します。定期的な作業を自動化できます。",
  },
  {
    icon: Settings2,
    title: "設定",
    description:
      "AI Providers、モデル、ツール権限、ネットワークなどの詳細設定を管理できます。",
  },
];

export function Tour({ onClose }: { onClose: () => void }) {
  const [current, setCurrent] = useState(0);
  const step = steps[current];
  const Icon = step.icon;

  return (
    <div
      className="mcp-install-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label="ツアー"
    >
      <div
        className="card mcp-install-panel"
        style={{ maxWidth: 480, padding: 32 }}
      >
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "flex-start",
          }}
        >
          <div>
            <p className="eyebrow">
              STEP {current + 1} OF {steps.length}
            </p>
            <h2>{t(step.title)}</h2>
          </div>
          <button
            type="button"
            className="icon"
            aria-label="閉じる"
            onClick={onClose}
          >
            <X size={19} />
          </button>
        </div>
        <div style={{ textAlign: "center", padding: "30px 0" }}>
          <Icon
            size={48}
            style={{ color: "var(--accent)", marginBottom: 16 }}
          />
          <p style={{ color: "var(--muted)", lineHeight: 1.7 }}>
            {t(step.description)}
          </p>
        </div>
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            marginTop: 20,
          }}
        >
          <Button variant="ghost" onClick={onClose}>
            {t("スキップ")}
          </Button>
          {current < steps.length - 1 ? (
            <Button onClick={() => setCurrent(current + 1)}>{t("次へ")}</Button>
          ) : (
            <Button onClick={onClose}>{t("完了")}</Button>
          )}
        </div>
        <div
          style={{
            display: "flex",
            justifyContent: "center",
            gap: 6,
            marginTop: 20,
          }}
        >
          {steps.map((_, i) => (
            <span
              key={i}
              style={{
                width: 8,
                height: 8,
                borderRadius: "50%",
                background: i === current ? "var(--accent)" : "var(--line)",
                transition: "background 0.15s",
              }}
            />
          ))}
        </div>
      </div>
    </div>
  );
}
