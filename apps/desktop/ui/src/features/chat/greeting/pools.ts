import type { Fragment, PartOfDay, Season, Tone } from "./types";

/**
 * 一文を登録します。文形は文末記号から必ず導きます。
 * 合成の制約は文形のタグで判定するため、ここで直接指定すると
 * タグと実文が食い違い、「？」「？」が連続しても弾けなくなります。
 */
export function sentence(
  id: string,
  text: string,
  tone: Tone = "polite",
  weight?: number,
): Fragment {
  return {
    id,
    text,
    tone,
    form: text.endsWith("？") ? "question" : "declarative",
    weight,
  };
}

// 1. 挨拶。時間帯ごとに語彙を分けるため約 48 文。
export const openings: Record<PartOfDay, readonly Fragment[]> = {
  earlyMorning: [
    sentence("em-1", "おはようございます。今日はどんな一日になりそうですか？"),
    sentence(
      "em-2",
      "おはようございます。朝のうちに決めると、あとで軽くなります。",
    ),
    sentence(
      "em-3",
      "おはようございます。一日のうちで最も静かな時間ですね。",
      "calm",
    ),
    sentence("em-4", "おはようございます。何から手をつきましょうか？"),
    sentence("em-5", "おはようございます。眠気は大丈夫ですか？"),
    sentence("em-6", "おはようございます。今は準備の時間ですかね？", "casual"),
  ],
  morning: [
    sentence("mo-1", "おはようございます。何をお手伝いしましょうか？"),
    sentence("mo-2", "おはようございます。今日は何から始めますか？"),
    sentence("mo-3", "おはようございます。一日が動き出す時間ですね。", "calm"),
    sentence("mo-4", "おはようございます。今あるものを整理して始めますか？"),
    sentence("mo-5", "おはようございます。今日の段取りを一緒に組みましょう。"),
    sentence(
      "mo-6",
      "おはようございます。何から手を入れましょうか？",
      "casual",
    ),
  ],
  lateMorning: [
    sentence("lm-1", "おはようございます。今はどのあたりまで進んでいますか？"),
    sentence("lm-2", "おはようございます。仕事の速度を上げられる時間です。"),
    sentence(
      "lm-3",
      "おはようございます。一番、判断が効く時間ですね。",
      "calm",
    ),
    sentence("lm-4", "おはようございます。今日中のゴールをどこにしますか？"),
    sentence("lm-5", "おはようございます。手を動かすのに十分な時間ですね。"),
    sentence("lm-6", "おはようございます。手を動かしますか？", "casual"),
  ],
  midday: [
    sentence("md-1", "こんにちは。昼のあいだに、ここから進めますか？"),
    sentence("md-2", "こんにちは。午前が一段落した時間ですね。", "calm"),
    sentence("md-3", "こんにちは。この時間に片づけたいことはありますか？"),
    sentence("md-4", "こんにちは。日中は判断が素直な時間帯です。"),
    sentence("md-5", "こんにちは。午後はどう使いますか？", "casual"),
    sentence("md-6", "こんにちは。一つ区切って進めましょう。"),
  ],
  afternoon: [
    sentence("af-1", "こんにちは。何をお手伝いできますか？"),
    sentence("af-2", "こんにちは。どんなことを進めましょうか？"),
    sentence("af-3", "こんにちは。午後は判断が細かい作業に向きます。", "calm"),
    sentence("af-4", "こんにちは。今日の続きから始めましょうか？"),
    sentence("af-5", "こんにちは。この時間のほうが頭が回ります。"),
    sentence("af-6", "こんにちは。何から手をつきますか？", "casual"),
  ],
  dusk: [
    sentence("dk-1", "こんにちは。今日はどんな一日でしたか？"),
    sentence("dk-2", "こんにちは。一日が静かに傾いていく時間ですね。", "calm"),
    sentence("dk-3", "こんにちは。帰る前に片づけたいことはありますか？"),
    sentence("dk-4", "こんにちは。残りの時間はどう使いますか？"),
    sentence("dk-5", "こんにちは。あと少しで終わる作業もありますか？"),
    sentence("dk-6", "こんにちは。夜のうちに決めておくと軽いです。"),
  ],
  evening: [
    sentence("ev-1", "こんばんは。何をお手伝いしましょうか？"),
    sentence("ev-2", "こんばんは。今日の締めくくりに何を進めますか？"),
    sentence("ev-3", "こんばんは。一日の終わりに整える時間ですね。", "calm"),
    sentence("ev-4", "こんばんは。終わらせたいことはありますか？"),
    sentence("ev-5", "こんばんは。今日の残りを片づけましょう。"),
    sentence("ev-6", "こんばんは。もう一息つけますか？", "casual"),
  ],
  lateNight: [
    sentence(
      "ln-1",
      "こんばんは。遅くまでお疲れさまです。何をお手伝いしましょうか？",
    ),
    sentence("ln-2", "こんばんは。夜は判断よりも、記録に向いています。"),
    sentence(
      "ln-3",
      "こんばんは。夜更けの作業は、慎重に進めるのが安全です。",
      "calm",
    ),
    sentence("ln-4", "こんばんは。明日に持ち越すものはありますか？"),
    sentence("ln-5", "こんばんは。眠気より優先したいことはありますか？"),
    sentence("ln-6", "こんばんは。静かなうちに片づけましょう。", "casual"),
  ],
};

// 2. 再訪時の挨拶。前回から一定時間あいているときだけ使う。
export const returningOpenings: readonly Fragment[] = [
  sentence("rt-1", "おかえりなさい。何から続けましょうか？"),
  sentence("rt-2", "おかえりなさい。続きから進めます。"),
  sentence("rt-3", "おかえりなさい。前回どこまでやっていましたか？"),
  sentence(
    "rt-4",
    "おかえりなさい。そのあいだに考えが変わったなら、そのまま教えてください。",
  ),
  sentence("rt-5", "おかえりなさい。また話せたのがうれしいです。", "casual"),
  sentence("rt-6", "おかえりなさい。今日は何を進めますか？"),
];

// 初回のみ、その日の最初の挨拶で使う。
export const firstOpenings: readonly Fragment[] = [
  sentence("ft-1", "はじめまして。最初に何をしましょうか？"),
  sentence("ft-2", "はじめまして。まずは結論からで構いません。", "calm"),
  sentence("ft-3", "はじめまして。ここから何を組み立てましょうか？"),
  sentence("ft-4", "はじめまして。記録は後からいつでも整理できます。"),
  sentence("ft-5", "はじめまして。今もっとも困っていることは何ですか？"),
  sentence("ft-6", "はじめまして。小さいところからでかまいません。", "casual"),
];

// 3. 状況節。常に断定で終えるので、どの挨拶のあとにも置ける。
export const weekdayContexts: readonly Fragment[] = [
  sentence("wd-1", "平日の時間です。"),
  sentence("wd-2", "少し集中したい時間ですね。", "calm"),
  sentence("wd-3", "区切りのよい場面です。"),
];

export const mondayContexts: readonly Fragment[] = [
  sentence("dm-1", "週末明けです。"),
  sentence("dm-2", "週の始まりなので、方針を決めやすい場面です。"),
  sentence("dm-3", "ゆっくり立ち上げる時間です。", "calm"),
];

export const fridayContexts: readonly Fragment[] = [
  sentence("fr-1", "週の締まりの時間です。"),
  sentence("fr-2", "週末を前に、あと少しですね。", "calm"),
  sentence("fr-3", "今日中のうちに決めておきたいことはありますか。"),
];

export const weekendContexts: readonly Fragment[] = [
  sentence("we-1", "週末のあいだにまとめて進めると楽です。"),
  sentence("we-2", "少し余裕のある時間ですね。", "calm"),
  sentence("we-3", "平日に回すと重い作業にちょうどいいです。"),
];

export const sundayContexts: readonly Fragment[] = [
  sentence("su-1", "日曜の静かな時間です。"),
  sentence("su-2", "週の境目なので、いったん整理したいですね。", "calm"),
  sentence("su-3", "平日とは別の進め方ができそうです。"),
];

export const morningEdgeContexts: readonly Fragment[] = [
  sentence("me-1", "朝のうちに決めると、あとで軽くなります。"),
  sentence("me-2", "まだ周囲は動いていない時間ですね。", "calm"),
  sentence("me-3", "今日いちばんの最初の時間です。"),
];

export const middayContexts: readonly Fragment[] = [
  sentence("ny-1", "昼のあいだは、区切りをつけやすい時間です。"),
  sentence(
    "ny-2",
    "目の前にあるのを片づけておくと、午後の作業が楽になります。",
    "calm",
  ),
  sentence("ny-3", "一息ついたところで、どこまで進めるか決めましょう。"),
];

export const nightContexts: readonly Fragment[] = [
  sentence("nc-1", "静かな時間に、思いついたことを片づけましょう。"),
  sentence("nc-2", "眠気と引き換えの作業になりかねません。", "calm"),
  sentence("nc-3", "長時間の作業、お疲れさまです。"),
];

export const lateNightContexts: readonly Fragment[] = [
  sentence("ll-1", "夜のうちに方針を決めておくと、あとで軽くなります。"),
  sentence("ll-2", "深夜は、判断よりも記録に向いています。", "calm"),
  sentence("ll-3", "眠る前に片づけられることはありますか。"),
];

export const seasonContexts: Record<Season, readonly Fragment[]> = {
  spring: [
    sentence("sp-1", "春の気配です。"),
    sentence("sp-2", "芽が出る時期なので、揃えにくい作業です。"),
    sentence(
      "sp-3",
      "春のうちに決めておくと、あとで動きやすくなります。",
      "calm",
    ),
  ],
  summer: [
    sentence("sm-1", "暑い時期なので、短い区切りで進めると無理がありません。"),
    sentence("sm-2", "夏のあいだは、予定は軽くしておくものです。"),
    sentence("sm-3", "日差しが強い時間ですね。", "calm"),
  ],
  autumn: [
    sentence("au-1", "秋の、余裕の出てきた時間です。"),
    sentence("au-2", "秋のあいだに整えておくと、年越しが軽くなります。"),
    sentence("au-3", "読みものの秋です。", "calm"),
  ],
  winter: [
    sentence("wi-1", "冬は、一年の締めの時間です。"),
    sentence("wi-2", "寒いあいだは、まとめて片づけやすい期間です。"),
    sentence("wi-3", "静かな季節です。", "calm"),
  ],
};

// 二十四節気（24 文）
export const solarTermContexts: Record<string, Fragment> = {
  小寒: sentence("st-1", "小寒です。"),
  大寒: sentence("st-2", "大寒です。"),
  立春: sentence("st-3", "立春です。"),
  雨水: sentence("st-4", "雨水です。"),
  啓蟄: sentence("st-5", "虫が動き出す時期です。"),
  春分: sentence("st-6", "春分です。"),
  清明: sentence("st-7", "清明です。"),
  穀雨: sentence("st-8", "穀雨です。"),
  立夏: sentence("st-9", "立夏です。"),
  小満: sentence("st-10", "小満です。"),
  芒種: sentence("st-11", "芒種です。"),
  夏至: sentence("st-12", "夏至です。"),
  小暑: sentence("st-13", "小暑です。"),
  大暑: sentence("st-14", "大暑です。"),
  立秋: sentence("st-15", "立秋です。"),
  処暑: sentence("st-16", "処暑です。"),
  白露: sentence("st-17", "白露です。"),
  秋分: sentence("st-18", "秋分です。"),
  寒露: sentence("st-19", "寒露です。"),
  霜降: sentence("st-20", "霜降です。"),
  立冬: sentence("st-21", "立冬です。"),
  小雪: sentence("st-22", "小雪です。"),
  大雪: sentence("st-23", "大雪です。"),
  冬至: sentence("st-24", "冬至です。"),
};

// 祝日（17 文）
export const holidayContexts: Record<string, Fragment> = {
  元日: sentence("hl-1", "元日です。"),
  成人の日: sentence("hl-2", "成人の日です。"),
  建国記念の日: sentence("hl-3", "建国記念の日です。"),
  天皇誕生日: sentence("hl-4", "天皇誕生日です。"),
  春分の日: sentence("hl-5", "春分の日です。"),
  昭和の日: sentence("hl-6", "昭和の日です。"),
  憲法記念日: sentence("hl-7", "憲法記念日です。"),
  みどりの日: sentence("hl-8", "みどりの日です。"),
  こどもの日: sentence("hl-9", "こどもの日です。"),
  山の日: sentence("hl-10", "山の日です。"),
  敬老の日: sentence("hl-11", "敬老の日です。"),
  秋分の日: sentence("hl-12", "秋分の日です。"),
  スポーツの日: sentence("hl-13", "スポーツの日です。"),
  文化の日: sentence("hl-14", "文化の日です。"),
  勤労感謝の日: sentence("hl-15", "勤労感謝の日です。"),
  振替休日: sentence("hl-16", "振替休日です。"),
  国民の休日: sentence("hl-17", "国民の休日です。"),
};

// 季節の行事（15 文）
export const eventContexts: Record<string, Fragment> = {
  年の始まり: sentence("ee-1", "年の始まりです。"),
  バレンタインデー: sentence("ee-2", "バレンタインデーです。"),
  ひな祭り: sentence("ee-3", "ひな祭りの日です。"),
  ホワイトデー: sentence("ee-4", "ホワイトデーです。"),
  エイプリルフール: sentence("ee-5", "エイプリルフールです。"),
  アースデイ: sentence("ee-6", "アースデイです。"),
  母の日: sentence("ee-7", "母の日です。"),
  父の日: sentence("ee-8", "父の日です。"),
  七夕: sentence("ee-9", "七夕です。"),
  お盆: sentence("ee-10", "お盆です。"),
  ハロウィン: sentence("ee-11", "ハロウィンです。"),
  ポインセチア: sentence("ee-12", "ポインセチアが飾られる季節です。"),
  クリスマスイブ: sentence("ee-13", "クリスマスイブです。"),
  クリスマス: sentence("ee-14", "クリスマスです。"),
  大晦日: sentence("ee-15", "大晦日です。"),
};

export const yearStartContexts: readonly Fragment[] = [
  sentence("ys-1", "年の出だしです。"),
  sentence(
    "ys-2",
    "年の初めなので、記録を浅く残しておくと、あとで作りやすいです。",
  ),
];

export const yearEndContexts: readonly Fragment[] = [
  sentence("ye-1", "年の終わりの時期です。"),
  sentence("ye-2", "年の締めに、決めたいことをひとつ決めておくと楽です。"),
];

// 直近の作業状況
export const activityContexts: Record<string, readonly Fragment[]> = {
  none: [
    sentence("ac-1", "まだスレッドがないので、はじめから整えられます。"),
    sentence("ac-2", "記録がまだないので、方針を自由に決められます。", "calm"),
    sentence("ac-3", "履歴が空です。最初のひとつから始めましょう。"),
  ],
  justNow: [
    sentence("aj-1", "たったいままで動いていた仕事です。"),
    sentence("aj-2", "続きをそのまま進められます。", "calm"),
    sentence("aj-3", "中断したところから、また始められます。"),
  ],
  today: [
    sentence("at-1", "今日もすでに進んでいます。"),
    sentence("at-2", "今日の流れは、そのまま続けていけます。", "calm"),
    sentence("at-3", "ここまで来れば、あと一息でまとまります。"),
  ],
  yesterday: [
    sentence("ay-1", "昨日の続きから進めると扱いやすいです。"),
    sentence("ay-2", "昨日の残りを引き継ぐなら、今がその続きです。", "calm"),
    sentence("ay-3", "昨日のログが残っています。"),
  ],
  thisWeek: [
    sentence("aw-1", "今週の記録が残っています。"),
    sentence("aw-2", "先週の続きがまだつながっています。"),
    sentence("aw-3", "週のあいだに開いておいた課題はありますか。", "calm"),
  ],
  earlier: [
    sentence("ae-1", "少し久しぶりの時間ですね。"),
    sentence(
      "ae-2",
      "記録が薄いぶん、はじめから組み直すほうが楽です。",
      "calm",
    ),
    sentence("ae-3", "前の記録は、あとから読み返せます。"),
  ],
};

// 前回からの訪問間隔
export const gapContexts: Record<string, readonly Fragment[]> = {
  reload: [sentence("gr-1", "画面を開き直した時間ですね。")],
  sameDay: [sentence("gs-1", "同じ日の続きですね。")],
  overnight: [
    sentence("go-1", "一晩空きましたね。"),
    sentence("go-2", "昨日の作業は、そのまま引き継げます。"),
  ],
  fewDays: [
    sentence("gf-1", "数日ぶりですね。"),
    sentence("gf-2", "少し離れましたが、記録は残っています。"),
  ],
  longAway: [
    sentence("gl-1", "少し久しぶりですね。"),
    sentence("gl-2", "記録はそのまま残っています。"),
  ],
  farAway: [
    sentence("ga-1", "久しぶりです。"),
    sentence("ga-2", "長いあいだ空きましたが、履歴は残っています。"),
  ],
};

export const workloadContexts: Record<string, readonly Fragment[]> = {
  empty: [
    sentence("wk-1", "まだ記録がないので、はじめから設計できます。"),
    sentence("wk-2", "何もない状態なので、制約を自由に決められます。", "calm"),
  ],
  light: [
    sentence("wk-3", "スレッドの数はまだ控えめです。"),
    sentence("wk-4", "少しの履歴があるので、すぐ始められそうです。"),
  ],
  steady: [
    sentence("wk-5", "積み上がった記録から、抜けを拾えます。"),
    sentence("wk-6", "この程度の量なら、まとめて整理できます。", "calm"),
  ],
  heavy: [
    sentence("wk-7", "記録が積み上がっています。"),
    sentence("wk-8", "これまでの流れを、まとめて見直せます。"),
  ],
};

// 4. 締め。「？」が続かないよう compose 側で制約する。
export const questionClosings: readonly Fragment[] = [
  sentence("qc-1", "何から始めましょうか？"),
  sentence("qc-2", "どこから手をつきましょうか？"),
  sentence("qc-3", "何をお手伝いしましょうか？"),
  sentence("qc-4", "今日の重点はありますか？"),
  sentence("qc-5", "最初の一歩はどこですか？"),
  sentence("qc-6", "何を進めたいですか？", "casual"),
  sentence("qc-7", "どこから始めますか？", "casual"),
  sentence("qc-8", "最初に片づけたいものはありますか？"),
];

export const offerClosings: readonly Fragment[] = [
  sentence("oc-1", "必要なところから始めましょう。"),
  sentence("oc-2", "一つずつ進めましょう。"),
  sentence("oc-3", "進めたいことを教えてください。"),
  sentence("oc-4", "最初の一歩を一緒に組みましょう。"),
  sentence("oc-5", "気が向くところからでかまいません。", "calm"),
  sentence("oc-6", "まったく別の話でもかまいません。", "calm"),
  sentence("oc-7", "準備ができたら教えてください。"),
  sentence("oc-8", "ひとこと動機があれば、そこから始められます。"),
];

export const togetherClosings: readonly Fragment[] = [
  sentence("tc-1", "今日も一緒に進めましょう。"),
  sentence("tc-2", "焦らなくても、順番にできます。", "calm"),
  sentence("tc-3", "少し遠回りでも、一緒に進めましょう。"),
  sentence("tc-4", "ここまで来れば、あとは進めるだけです。"),
];

export const resumeClosings: readonly Fragment[] = [
  sentence("rc-1", "昨日の続きから始めましょうか？"),
  sentence("rc-2", "前回どこまでやっていましたか？"),
  sentence("rc-3", "中断したところから、そのまま再開できます。"),
  sentence("rc-4", "続きを開くところから始めましょう。"),
];

// 5. 見出しの補足。断定で終え、ShieldCheck と並べて表示する。
export const notes: Record<string, readonly Fragment[]> = {
  default: [
    sentence("nt-1", "必要なときだけ、確認のうえで作業を進めます"),
    sentence("nt-2", "実行前に、何をするかを必ず説明します。"),
    sentence("nt-3", "破壊的な操作は、必ず確認を挟みます。"),
    sentence("nt-4", "判断が分かれるところは、あなたに訊きます。"),
    sentence("nt-5", "進める前に、今やることを説明します。", "calm"),
    sentence("nt-6", "記録はローカルに保存し、必要な範囲だけ共有します。"),
  ],
  empty: [
    sentence("nt-7", "最初の一歩は、必ず確認してから始めます。"),
    sentence("nt-8", "ツールを使うときは、許可された範囲に限ります。"),
    sentence("nt-9", "記録が少ないぶん、進め方は一緒に決められます。", "calm"),
  ],
  heavy: [
    sentence("nt-10", "記録が多いぶん、抜けがないか一緒に確認します。"),
    sentence("nt-11", "古い記録は消さずに、あとから読み返せます。"),
    sentence("nt-12", "既存の記録を壊さない順番で進めます。"),
  ],
  night: [
    sentence("nt-13", "長時間の作業になるため、区切りを明確にします。"),
    sentence("nt-14", "眠気の方が先にくる前に、区切りをつけます。"),
  ],
  returning: [
    sentence("nt-15", "前回の続きとして、記録と並べて進めます。"),
    sentence("nt-16", "変更点は、あとから追えるように残します。"),
  ],
  // Desktop 版はローカルでファイルを書き換えられるため、補足も Desktop の
  // 実行モデル（Tool ごとの承認と差分レビュー）に合わせて書き下す。
  agent: [
    sentence(
      "nt-17",
      "ファイルの読み書きとシェル実行は、実行のたびに確認が出ます。",
    ),
    sentence("nt-18", "編集は差分として残し、レビューできる状態にします。"),
    sentence("nt-19", "検証のあとでも読み返せるように残します。", "calm"),
  ],
  project: [
    sentence("nt-20", "対象は、プロジェクトに設定したフォルダの中だけです。"),
    sentence("nt-21", "フォルダの外へは、書き込まずに済みます。"),
  ],
};

// Desktop 固有の状況節。プロジェクト・サーフェス・差分の状態に応じて選びます。
export const projectContexts: readonly Fragment[] = [
  sentence("pc-1", "プロジェクトのフォルダが対象です。"),
  sentence("pc-2", "指示はプロジェクトごとに分かれています。"),
  sentence("pc-3", "フォルダの中のファイルなら、どこでも確認できます。"),
];

export const noProjectContexts: readonly Fragment[] = [
  sentence("np-1", "プロジェクトは未設定です。"),
  sentence("np-2", "フォルダを決めれば、対象を絞れます。"),
  sentence("np-3", "任意のフォルダから始められます。"),
];
