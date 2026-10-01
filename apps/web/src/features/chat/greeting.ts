export type TimeOfDay = "morning" | "afternoon" | "evening" | "night";

export function timeOfDay(date: Date = new Date()): TimeOfDay {
  const hour = date.getHours();
  if (hour >= 5 && hour < 11) return "morning";
  if (hour >= 11 && hour < 18) return "afternoon";
  if (hour >= 18 && hour < 22) return "evening";
  return "night";
}

export const greetings: Record<TimeOfDay, readonly string[]> = {
  morning: [
    "おはようございます。何をお手伝いしましょうか？",
    "おはようございます。今日は何から始めますか？",
    "おはようございます。どんな作業を進めましょうか？",
    "おはようございます。今日も一緒に進めましょう。",
  ],
  afternoon: [
    "こんにちは。何をお手伝いできますか？",
    "こんにちは。どんなことを進めましょうか？",
    "こんにちは。今日の続きから始めましょうか？",
    "こんにちは。気になることを聞かせてください。",
  ],
  evening: [
    "こんばんは。何をお手伝いしましょうか？",
    "こんばんは。今日の締めくくりに何を進めますか？",
    "こんばんは。どんな作業をお手伝いしましょうか？",
    "こんばんは。今日はどんな一日でしたか？",
  ],
  night: [
    "こんばんは。遅くまでお疲れさまです。何をお手伝いしましょうか？",
    "こんばんは。夜間の作業ですね。何から始めますか？",
    "こんばんは。静かな時間に、何を進めましょうか？",
    "こんばんは。落ち着いて進められるようお手伝いします。",
  ],
};

export function randomGreeting(random: () => number = Math.random, date: Date = new Date()): string {
  const list = greetings[timeOfDay(date)];
  return list[Math.floor(random() * list.length)] ?? list[0];
}
