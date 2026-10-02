import { workloadOf } from "./signals";
import type { Signals, Suggestion } from "./types";

interface Candidate extends Suggestion {
  weight: number;
}

/**
 * 画面のうらに実在する機能だけに対応する提案に絞ります。
 * 文言はすべて翻訳キーになるため、時間帯や祝日から文字列を組み立てず静的に保ちます。
 */
function buildCandidates(signals: Signals): Candidate[] {
  const candidates: Candidate[] = [];

  if (signals.activity === "yesterday" || signals.activity === "today")
    candidates.push({
      id: "sg-resume",
      label: "昨日の続きを進める",
      prompt: "前回の会話を確認して、途中から続きを進めてください。",
      weight: 4,
    });

  // 会話が 0 件の時だけ提案します。記録があっても最終活動タイムが
  // 読めない場合はあり得るので、会話数で判定します。
  if (signals.conversationCount === 0)
    candidates.push({
      id: "sg-first",
      label: "最初の会話をつくる",
      prompt: "MIX agent の使い方を、実際の場面で教えてください。",
      weight: 4,
    });

  if (workloadOf(signals.conversationCount) === "heavy")
    candidates.push({
      id: "sg-tidy",
      label: "最近の会話を整理する",
      prompt: "最近の会話を一覧して、タイトルと内容がずれているものを提案してください。",
      weight: 4,
    });

  if (workloadOf(signals.conversationCount) === "steady")
    candidates.push({
      id: "sg-review",
      label: "記録の抜けを洗い出す",
      prompt: "最近の会話を一覧して、続きが途切れているものを洗い出してください。",
      weight: 3,
    });

  if (signals.projectCount > 0)
    candidates.push({
      id: "sg-project",
      label: "プロジェクトの指示を見直す",
      prompt: "プロジェクトの指示を一覧し、抜けや矛盾がないか確認してください。",
      weight: 3,
    });

  if (signals.agentCount > 0)
    candidates.push({
      id: "sg-agent",
      label: "アシスタントを使い分ける",
      prompt: "保存済みのアシスタントを一覧して、用途ごとの使い分けを提案してください。",
      weight: 2,
    });

  if (signals.pinnedCount > 0)
    candidates.push({
      id: "sg-pinned",
      label: "ピン留めした会話をまとめる",
      prompt: "ピン留めした会話を一覧して、関連を整理してください。",
      weight: 2,
    });

  if (signals.partOfDay === "lateNight" || signals.partOfDay === "evening")
    candidates.push({
      id: "sg-daylog",
      label: "今日の作業ログをまとめる",
      prompt: "今日の会話を要約して、作業ログの形にまとめてください。",
      weight: 3,
    });

  if (signals.holiday || signals.event || signals.solarTerm)
    candidates.push({
      id: "sg-season",
      label: "季節の予定を整理する",
      prompt: "季節の行事や予定を整理して、次の進め方を提案してください。",
      weight: 2,
    });

  candidates.push(
    {
      id: "sg-research",
      label: "Webを調べてレポートを作る",
      prompt: "Webを調べて、出典つきのレポートを作成してください。",
      weight: 3,
    },
    {
      id: "sg-memory",
      label: "好みを Memory として残す",
      prompt: "今回覚えた好みを、Memory として保存してください。",
      weight: 2,
    },
    {
      id: "sg-schedule",
      label: "定期実行を設定する",
      prompt: "毎日繰り返したい作業を、定期実行として設定してください。",
      weight: 2,
    },
  );
  return candidates;
}

/** シグナルから提案を重み付き抽選で選びます。同じ提案は二回続けて出しません。 */
export function suggestionsFor(
  signals: Signals,
  recent: readonly string[] = [],
  count = 3,
  random: () => number = Math.random,
): Suggestion[] {
  const all = buildCandidates(signals);
  const seen = new Set(recent);
  // 直近の提案は先に候補から外します。外し切った分だけ後から補填して、
  // 画面上の枚数が履歴の溜まり具合で減らないようにします。
  const unseen = all.filter((item) => !seen.has(item.id));
  const order = [
    ...weightedOrder(unseen, random),
    ...weightedOrder(
      all.filter((item) => seen.has(item.id)),
      random,
    ),
  ];
  return order.slice(0, count).map((item) => ({ id: item.id, label: item.label, prompt: item.prompt }));
}

function weightedOrder(pool: readonly Candidate[], random: () => number): Candidate[] {
  const remaining = [...pool];
  const ordered: Candidate[] = [];
  while (remaining.length) {
    const total = remaining.reduce((sum, item) => sum + item.weight, 0);
    let threshold = random() * total;
    let index = remaining.length - 1;
    for (let i = 0; i < remaining.length; i++) {
      threshold -= remaining[i].weight;
      if (threshold < 0) {
        index = i;
        break;
      }
    }
    const [chosen] = remaining.splice(index, 1);
    ordered.push(chosen);
  }
  return ordered;
}