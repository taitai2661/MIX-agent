import type { Signals, Suggestion } from "./types";

interface Candidate extends Suggestion {
  weight: number;
}

/**
 * 画面のうらに実在する機能だけに対応する提案に絞ります。
 * 文言はすべて翻訳キーになるため、時間帯や祝日から文字列を組み立てず静的に保ちます。
 *
 * Desktop 版はローカルでファイルを読み書きし、ブラウザを操作し、差分を残す
 * ため、Docker 版面向の提案は使いません。既存のクイックプロンプト 4 件も
 * ここに組み込むことで、i18n の孤立キーを増やします。
 */
function buildCandidates(signals: Signals): Candidate[] {
  const candidates: Candidate[] = [];
  const agent = signals.surface === "desktop-agent";

  // 既存のクイックプロンプト（Transcript.tsx の QUICK_PROMPTS 相当）。
  // agent サーフェスでは思考モードの指示が使えないため、出し分ける。
  candidates.push({
    id: "sg-explain",
    label: "このコードの意図と動作を、日本語で要約して",
    prompt: "このコードの意図と動作を、日本語で要約して",
    mode: agent ? undefined : "thinking",
    weight: 3,
  });
  candidates.push({
    id: "sg-refactor",
    label: "リファクタ案を出して。安全策もセットで",
    prompt: "リファクタ案を出して。安全策もセットで",
    weight: 3,
  });
  candidates.push({
    id: "sg-todos",
    label: "リポジトリ内の TODO を全部集めて",
    prompt: "リポジトリ内の TODO を全部集めて",
    weight: 2,
  });
  candidates.push({
    id: "sg-readme",
    label: "このプロジェクトの README 草案を書いて",
    prompt: "このプロジェクトの README 草案を書いて",
    weight: 2,
  });

  if (agent) {
    candidates.push(
      {
        id: "sg-diff",
        label: "直前の変更を確認して",
        prompt:
          "いま残っている変更を差分で見せてください。意図と合致しているか確認します。",
        weight: 4,
      },
      {
        id: "sg-tests",
        label: "テストを実行して結果を見る",
        prompt: "関連するテストを実行して、失敗があれば原因を調べてください。",
        weight: 4,
      },
      {
        id: "sg-browser",
        label: "ブラウザで表示を確認する",
        prompt: "ブラウザで表示して、意図通りか確認してください。",
        weight: 3,
      },
    );
  } else {
    candidates.push({
      id: "sg-plan",
      label: "手順に分けて示して",
      prompt: "これからやりたいことを、手順に分けて示してください。",
      weight: 3,
    });
  }

  if (signals.projectActive) {
    candidates.push(
      {
        id: "sg-instructions",
        label: "プロジェクトの指示を読み直す",
        prompt:
          "このプロジェクトの指示を一覧し、抜けや矛盾がないか確認してください。",
        weight: 4,
      },
      {
        id: "sg-folder",
        label: "フォルダ構成を教えて",
        prompt: "このフォルダの構成を、一覧で教えてください。",
        weight: 3,
      },
    );
  } else {
    candidates.push({
      id: "sg-project",
      label: "プロジェクトを作る",
      prompt: "このフォルダを、プロジェクトの作業場所として設定してください。",
      weight: 3,
    });
  }

  if (signals.activity === "yesterday" || signals.activity === "today")
    candidates.push({
      id: "sg-resume",
      label: "昨日の続きを進める",
      prompt: "前回のスレッドを確認して、途中から続きを進めてください。",
      weight: 4,
    });

  if (signals.activity === "none" || signals.conversationCount === 0)
    candidates.push({
      id: "sg-first",
      label: "最初のスレッドをつくる",
      prompt: "MIX-agent の使い方を、実際の場面で教えてください。",
      weight: 4,
    });

  if (signals.conversationCount >= 20)
    candidates.push({
      id: "sg-tidy",
      label: "最近のスレッドを整理する",
      prompt:
        "最近のスレッドを一覧して、タイトルと内容がずれているものを提案してください。",
      weight: 3,
    });

  if (signals.partOfDay === "lateNight" || signals.partOfDay === "evening")
    candidates.push({
      id: "sg-daylog",
      label: "今日の作業ログをまとめる",
      prompt: "今日のスレッドを要約して、作業ログの形にまとめてください。",
      weight: 3,
    });

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
  const order = [
    ...weightedOrder(
      all.filter((item) => !seen.has(item.id)),
      random,
    ),
    ...weightedOrder(
      all.filter((item) => seen.has(item.id)),
      random,
    ),
  ];
  return order.slice(0, count).map((item) => ({
    id: item.id,
    label: item.label,
    prompt: item.prompt,
    mode: item.mode,
  }));
}

function weightedOrder(
  pool: readonly Candidate[],
  random: () => number,
): Candidate[] {
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
