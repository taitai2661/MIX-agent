/// Display-language switcher, mirroring the Docker edition's approach
/// (`docker/MIX-agent/apps/web/src/app/i18n.ts`): Japanese strings are the
/// canonical keys and `t()` maps them to English when the UI language is `en`.

export type Language = "ja" | "en";

const STORAGE_KEY = "mix-agent-language";

export function browserLanguage(
  languages: readonly string[] = typeof navigator === "undefined" ? [] : navigator.languages,
): Language {
  for (const value of languages) {
    if (value.toLowerCase().startsWith("ja")) return "ja";
    if (value.toLowerCase().startsWith("en")) return "en";
  }
  return "ja";
}

let language: Language = initialLanguage();

function initialLanguage(): Language {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "ja" || stored === "en") return stored;
  } catch {
    // localStorage unavailable — fall through to the browser default.
  }
  return browserLanguage();
}

export function getLanguage(): Language {
  return language;
}

export function setLanguage(value: Language): void {
  if (language === value) return;
  language = value;
  try {
    localStorage.setItem(STORAGE_KEY, value);
  } catch {
    // Persisting is best-effort; the in-memory switch still applies.
  }
  if (typeof document !== "undefined") document.documentElement.lang = value;
}

if (typeof document !== "undefined") document.documentElement.lang = language;

const en: Record<string, string> = {
  // Shell / sidebar
  "新規スレッド": "New thread",
  "スレッドを検索": "Search threads",
  "スレッドがありません": "No threads yet",
  "新しい会話を始めましょう": "Start a new conversation",
  "今日": "Today",
  "昨日": "Yesterday",
  "過去7日": "Previous 7 days",
  "それより前": "Older",
  "ワークスペース": "Workspace",
  "設定": "Configure",
  "ナレッジ": "Knowledge",
  "システム": "System",
  "チャット": "Chat",
  "プロバイダー": "Providers",
  "モデル": "Models",
  "エージェント": "Agents",
  "ツール": "Tools",
  "承認": "Approvals",
  "ブラウザ": "Browser",
  "メモリ": "Memory",
  "スキル": "Skills",
  "プロジェクト": "Projects",
  "スケジュール": "Schedules",
  "診断": "Diagnostics",
  "名前を変更": "Rename",
  "削除": "Delete",
  "エクスポート": "Export",
  "このスレッドを削除しますか？": "Delete this thread?",
  "言語": "Language",
  "テーマ": "Theme",
  "ライト": "Light",
  "ダーク": "Dark",
  "システムに従う": "Follow system",
  "コマンドパレット": "Command palette",
  "コマンドを検索…": "Search commands…",

  // Chat composer
  "MIX-agent に話しかける…": "Message MIX-agent…",
  "深く考えてほしいことを入力…": "Ask for a deep, careful answer…",
  "複雑な作業を依頼…": "Describe a complex task…",
  "プロジェクトを選択": "Select a project",
  "送信": "Send",
  "停止": "Stop",
  "画像を添付": "Attach image",
  "音声入力": "Voice input",
  "モデルが未設定です。先にプロバイダーを追加してください。": "No model configured. Add a provider first.",
  "モデル未設定": "No model selected",
  "画像を添付するにはVision対応モデルを選択してください": "Select a vision-capable model to attach images",
  "このモデルは画像入力に対応していません": "This model does not support image input",
  "添付ファイルが大きすぎます (上限8MB)": "Attachment is too large (8 MB limit)",
  "画像は送信されません": "Images will not be sent",
  "Enter で送信 · Shift+Enter で改行": "Enter to send · Shift+Enter for newline",

  // Transcript
  "まだメッセージがありません": "No messages yet",
  "会話を選択するか、新規スレッドを作成してください": "Select a conversation or create a new thread",
  "何でも質問してください。/ でコマンド、⌘K で検索。": "Ask anything. / for commands, ⌘K to search.",
  "思考プロセス": "Reasoning",
  "思考中…": "Thinking…",
  "実行中…": "Running…",
  "ツール実行": "Tool activity",
  "件のツール": "tools",
  "ステップ": "steps",
  "差分": "Diff",
  "コピー": "Copy",
  "コピーしました": "Copied",
  "詳細": "Details",
  "引数": "Arguments",
  "結果": "Result",
  "最新へ": "Jump to latest",
  "チェックポイント": "Checkpoints",
  "再開": "Resume",
  "計画": "Plan",
  "変更の確認中": "Verifying a change",
  "検証に失敗しました": "Verification failed",
  "この会話をMarkdownで書き出す": "Export this conversation as Markdown",

  // Approvals / banners
  "承認が必要です": "Approval required",
  "今回のみ許可": "Allow once",
  "常に許可": "Always allow",
  "拒否": "Deny",
  "予算を使い切りました": "Budget exhausted",
  "延長する": "Extend",
  "ここで終了する": "End here",
  "停滞を検出しました": "Stagnation detected",
  "検証が必要です": "Verification required",
  "破壊的ツール {tool} が実行されました。確認の読み取りが期待されます。": "Destructive tool {tool} fired; a confirming read is expected.",
  "ステップ上限 {steps} / ツール上限 {calls} に到達しました。": "Reached the step limit ({steps}) / tool limit ({calls}).",

  // Tool activity
  "このRunのアクティビティ": "Run activity",
  "アクティビティはありません": "No activity",
  "実行中": "Running",
  "成功": "Success",
  "失敗": "Failed",
  "新規ファイル": "New file",
  "行追加": "lines added",
  "行削除": "lines removed",
  "もっと見る": "Show more",
  "閉じる": "Close",

  // Generic
  "保存": "Save",
  "キャンセル": "Cancel",
  "作成": "Create",
  "検索": "Search",
  "読み込み中…": "Loading…",
  "エラー": "Error",
  "初期設定": "Initial setup",
  "プロバイダーを追加": "Add provider",
  "未設定": "Not set",
  "保存しました": "Saved",
  "無効": "Disabled",
  "有効": "Enabled",
  "はい": "Yes",
  "いいえ": "No",
  "更新": "Refresh",
  "戻る": "Back",

  // Projects / file tree / changes
  "ファイルツリー": "File tree",
  "隠しファイルを表示": "Show hidden files",
  "隠しファイルを隠す": "Hide hidden files",
  "プロジェクトにフォルダを設定してください": "Set a folder on a project first",
  "git を利用できません": "git is not available",
  "変更": "Changes",
  "変更はありません": "No changes",
  "差分はありません": "No differences",
  "未追跡ファイルは差分に含まれません": "Untracked files are not part of a diff",
  "git リポジトリではありません": "Not a git repository",
  "作業ツリー": "Working tree",
  "ステージ済み": "Staged",
  "未追跡": "Untracked",
  "追加": "Added",
  "削除しました": "Deleted",
  "改名": "Renamed",
  "バイナリファイル": "Binary file",
  "ファイルが大きすぎるため冒頭のみ表示します": "File too large; showing the beginning only",
  "フォルダを選択": "Choose folder",
  "資料": "Sources",
  "資料を追加": "Add source",
  "紐づく会話": "Linked chats",
  "このプロジェクトで新しいチャット": "New chat in this project",
  "プロジェクトを削除しますか？ 会話は残ります。": "Delete this project? Chats are kept.",
  "指示を保存": "Save instructions",
  "名前": "Name",
  "指示": "Instructions",
  "会話がありません": "No chats yet",
  "プロジェクトなし": "No project",
  "隠しファイル": "Hidden files",
  "フォルダ": "Folder",
  "解除": "Clear",
  "このフォルダがエージェントの作業ディレクトリになります": "This folder becomes the agent's working directory",
  "を検出すると自動で読み込みます": "is picked up automatically when present",
  "プロジェクトへの指示…": "Instructions for this project…",
  "資料テキストを貼り付け…": "Paste reference text…",
  "プロジェクトフィルタを解除": "Clear project filter",
  "プロジェクト単位の資料と指示を管理します。": "Group chats, instructions and reference texts by project.",
  "（空）": "(empty)",

  // Settings pane tabs / groups
  "アプリ": "App",
  "構成": "Configuration",
  "外観": "Appearance",
  "一般": "General",
  "ネットワーク": "Network",
  "アップデート": "Updates",
  "バックアップ": "Backup",
};

/** Translate a Japanese UI key into the active display language. */
export function t(value: string): string {
  if (language === "ja") return value;
  return en[value] ?? value;
}

/** `t` with `{placeholder}` interpolation. */
export function tf(value: string, params: Record<string, string | number>): string {
  let out = t(value);
  for (const [key, replacement] of Object.entries(params)) {
    out = out.split(`{${key}}`).join(String(replacement));
  }
  return out;
}
