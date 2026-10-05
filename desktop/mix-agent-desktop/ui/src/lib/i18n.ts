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

export const en: Record<string, string> = {
  // Shell / sidebar
  "新規スレッド": "New thread",
  "スレッドを検索": "Search threads",
  "スレッドがありません": "No threads yet",
  "一致するスレッドがありません": "No threads match your search",
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
  "選択したモデルが利用できません": "Selected model is unavailable",
  "選択したモデルのプロバイダーが削除されています。モデルを再選択してください。": "The provider for the selected model has been deleted. Please pick another model.",
  "画像を添付するにはVision対応モデルを選択してください": "Select a vision-capable model to attach images",
  "このモデルは画像入力に対応していません": "This model does not support image input",
  "添付ファイルが大きすぎます (上限8MB)": "Attachment is too large (8 MB limit)",
  "画像は送信されません": "Images will not be sent",
  "Enter で送信 · Shift+Enter で改行": "Enter to send · Shift+Enter for newline",
  "モード": "Mode",
  "改行": "newline",
  "文字": "chars",
  "プロジェクトがありません": "No projects yet",
  "深い思考": "Think deeper",
  "クリア": "Clear",
  "プロジェクト紐付けあり": "Linked to a project",
  "サイドバーの幅": "Sidebar width",
  "サイドバーの表示切り替え": "Toggle sidebar",
  "ファイルツリーの幅": "File tree width",
  "サイドパネルの幅": "Side panel width",

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
  "実行に失敗しました": "The run failed",
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
  "ローカル LLM を検出": "Detect local LLMs",
  "LM Studio や Ollama などのローカル推論サーバーを自動で探します。": "Automatically look for local inference servers such as LM Studio or Ollama.",
  "この PC で検出できるランタイムはありません。下の手動指定か、上のプリセットから追加してください。": "No runtimes were detected on this machine. Use the manual field below, or add a preset above.",
  "Swagger や任意のポートで起動中のサーバーも指定できます。例: http://192.168.1.10:11434/v1": "Also accepts a server on a custom port or a network address. e.g. http://192.168.1.10:11434/v1",
  "取り込む": "Import",
  "取り込み済み": "Imported",
  "プロバイダーとして追加してモデルを登録": "Register as a provider and add its models",
  "モデルが 1 つもロードされていません": "No models are loaded",
  "起動してください": "Start the server",
  "未起動": "Not running",
  "モデル一覧を取得": "Fetch model list",
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
  "新規プロジェクト": "New project",
  "プロジェクトを作成": "Create project",
  "プロジェクト名": "Project name",
  "指示を保存": "Save instructions",
  "名前": "Name",
  "未入力の場合はフォルダ名が使われます": "Left blank, the folder name is used",
  "指示": "Instructions",
  "会話がありません": "No chats yet",
  "プロジェクトなし": "No project",
  "隠しファイル": "Hidden files",
  "フォルダ": "Folder",
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

  // Surfaces (top-level switcher)
  "chat": "chat",
  "desktop-agent": "desktop-agent",
  "作成、学習、探索": "Create, learn, explore",
  "ビルド、デバッグ、リリース": "Build, debug, release",
  "サーフェスを切り替え": "Switch surface",
  "すべてのスレッド": "All threads",
  "サーフェスで絞り込み": "Filter by surface",

  // Keys referenced from files owned elsewhere (App.tsx, LocalDiscoveryCard,
  // ProvidersPane, McpPane, …) — registered here so English never falls back
  // to the Japanese original.
  "アクティビティを表示": "Show activity",
  "サイドパネルを閉じる": "Close side panel",
  "プロバイダーが未設定です": "No providers configured",
  "接続に失敗しました": "Connection failed",
  "OAuth に失敗しました": "OAuth failed",
  "検出": "Detect",
  "テスト": "Test",
  "MCP": "MCP",

  // relativeTime (lib/format.ts)
  "たった今": "Just now",
  "{n}分前": "{n} min ago",
  "{n}時間前": "{n}h ago",
  "{n}日前": "{n}d ago",

  // Composer modes (translated at the render site via `t(mode.label)`)
  "思考": "Thinking",
  "素早い質疑応答": "Quick replies",
  "深く考えて回答": "Answer with deeper reasoning",
  "ツールで自律的に作業": "Work autonomously with tools",

  // Knowledge pane
  "ファイル名 (例: architecture.md)": "Filename (e.g. architecture.md)",
  "ファイルから": "From file",
  "ここに本文を貼り付け…": "Paste text here…",
  "追加する": "Add",
  "キーワード": "Keyword",
  "該当なし": "No matches",
  "登録済みドキュメント": "Registered documents",
  "まだドキュメントがありません。": "No documents yet.",
  "ファイル名": "Filename",
  "チャンク": "Chunks",
  "登録日時": "Added at",

  // Memory pane
  "Memory を追加": "Add memory",
  "タイトル": "Title",
  "本文…": "Body…",
  "agent は毎回の Run 開始時に Memory を検索し、関連するものをコンテキストへ注入します。":
    "The agent searches Memory at the start of every run and injects the relevant entries into context.",
  "キーワード (例: rust, deploy, postgres)": "Keyword (e.g. rust, deploy, postgres)",
  "登録済み Memory": "Saved memories",
  "まだ Memory がありません。": "No memories yet.",
  "Mark as still valid": "Mark as still valid",
  "Search instead of editing": "Search instead of editing",
  "Revision history": "Revision history",
  "No revisions yet.": "No revisions yet.",
  "restore": "restore",
  "close": "close",
  "verify": "verify",

  // Tools pane
  "まだ登録されていません。": "Nothing registered yet.",
  "説明": "Description",
  "既定権限": "Default permission",
  "権限ルール": "Permission rules",
  "スコープ付きルール": "Scoped rules",
  "読み取りは許可、書き込み・削除は承認": "Reads are allowed; writes and deletes ask for approval",

  // Approvals pane
  "承認待ちのアクションはありません": "No actions are waiting for approval",
  "チャット内でもインラインで承認できます。": "You can also approve inline from the chat.",

  // Agents pane
  "ツールを絞った専用エージェントを定義します。":
    "Define dedicated agents restricted to a subset of tools.",
  "Agent 名": "Agent name",
  "Name": "Name",
  "System instructions": "System instructions",
  "System instructions…": "System instructions…",

  // Projects pane
  "Project 名": "Project name",
  "まだありません。": "Nothing here yet.",

  // Models pane
  "ラベル": "Label",
  "Provider": "Provider",
  "Context": "Context",
  "Vision": "Vision",
  "Tools": "Tools",
  "Reasoning": "Reasoning",

  // Diff preview
  "binary or unreadable old content": "binary or unreadable old content",

  // Shared components / error boundary
  "Copy": "Copy",
  "The interface failed to start": "The interface failed to start",
  "Reload": "Reload",

  // Diagnostics pane
  "Storage": "Storage",
  "Secrets (Keychain)": "Secrets (Keychain)",
  "Engine": "Engine",
  "Counts": "Counts",
  "OK": "OK",
  "Fail": "Fail",
  "Providers": "Providers",
  "Models": "Models",
  "Memories": "Memories",
  "Skills": "Skills",
  "Active Runs": "Active Runs",
  "Database Encryption": "Database Encryption",
  "Encrypted at rest": "Encrypted at rest",
  "Not encrypted (in-memory + Keychain only)": "Not encrypted (in-memory + Keychain only)",
  "Encrypted": "Encrypted",
  "Pending": "Pending",
  "Master key is present in the OS Keychain": "Master key is present in the OS Keychain",
  "Master key will be created on first use": "Master key will be created on first use",
  "Keychain status unknown": "Keychain status unknown",
  "Database at-rest encryption is not enabled in this build. Conversation logs, run history and tool arguments are stored in plaintext; rely on disk-level encryption (FileVault / BitLocker / LUKS) until the SQLCipher migration lands.": "Database at-rest encryption is not enabled in this build. Conversation logs, run history and tool arguments are stored in plaintext; rely on disk-level encryption (FileVault / BitLocker / LUKS) until the SQLCipher migration lands.",

  // Browser pane
  "Backend": "Backend",
  "Refresh status": "Refresh status",
  "Refresh": "Refresh",
  "headless": "headless",
  "headed": "headed",
  "profiles": "profiles",
  "artifacts": "artifacts",
  "No Chromium binary resolved. Install a system Chromium or bundle one to enable the Browser.":
    "No Chromium binary resolved. Install a system Chromium or bundle one to enable the Browser.",
  "Workspace profile": "Workspace profile",
  "new profile": "new profile",
  "(not provisioned)": "(not provisioned)",
  "Reset": "Reset",
  "Action": "Action",
  "Activity": "Activity",
  "No actions yet.": "No actions yet.",
  "ok": "ok",
  "error": "error",
  "output": "output",
  "Known profiles": "Known profiles",
  "ready": "ready",
  "empty": "empty",
  "Each workspace keeps its own cookies, localStorage and cache, matching the agent's workspace.":
    "Each workspace keeps its own cookies, localStorage and cache, matching the agent's workspace.",
  "Page info": "Page info",
  "Title, URL and viewport": "Title, URL and viewport",
  "Navigate": "Navigate",
  "Load a URL": "Load a URL",
  "DOM": "DOM",
  "innerHTML of a selector (or the body)": "innerHTML of a selector (or the body)",
  "Click": "Click",
  "Click a CSS selector": "Click a CSS selector",
  "Type": "Type",
  "Type text into a selector": "Type text into a selector",
  "Scroll": "Scroll",
  "Scroll by dx/dy": "Scroll by dx/dy",
  "Wait": "Wait",
  "Wait for selector, URL or JS": "Wait for selector, URL or JS",
  "Screenshot": "Screenshot",
  "Capture the viewport": "Capture the viewport",
  "Evaluate": "Evaluate",
  "Run JavaScript": "Run JavaScript",
  "CSS selector, e.g. #search or body": "CSS selector, e.g. #search or body",
  "text to type": "text to type",
  "wait until URL contains…": "wait until URL contains…",

  // Health indicator / update banner
  "正常": "OK",
  "接続エラー": "Connection error",
  "確認中": "Checking…",
  "ヘルス": "Health",
  "アップデートがあります": "Update available",
  "インストールして再起動": "Install & restart",

  // Command palette
  "一致する項目がありません": "No matching commands",

  // Settings: quick prompts, network, updates, mode select
  "Egress 許可リスト": "Egress allowlist",
  "未設定時は Tool がすべての公開ドメインへ到達可能。ワイルドカード (*) と完全一致ドメインを混在可。": "With no list, tools can reach any public domain. Wildcard (*) and exact hosts can be mixed.",
  "自動更新を有効化": "Enable automatic updates",
  "匿名の利用統計を共有 (将来)": "Share anonymous usage statistics (coming soon)",

  // Backup / restore, MCP OAuth, skills export, sidebar landmark
  "サイドバー": "Sidebar",
  "バックアップ / 復元": "Backup / restore",
  "データベース全体をスナップショットし、圧縮・暗号化します (Argon2id + AES-256-GCM)。API キーは OS キーチェーンに残るため含まれません。":
    "The whole database is snapshotted, compressed and encrypted (Argon2id + AES-256-GCM). API keys stay in the OS Keychain and are not included.",
  "バックアップパスワード": "Backup password",
  "暗号化バックアップを作成": "Create an encrypted backup",
  "復元する既存の .mixbak": "Existing .mixbak to restore",
  "バックアップを検証し、再起動時に復元": "Verify a backup and reload it on restart",
  "復元": "Restore",
  "今すぐ再起動": "Restart now",
  "{bytes} バイト（{tables} テーブル）を {path} に保存しました":
    "Saved {bytes} bytes ({tables} tables) to {path}",
  "{tables} テーブルを検証しました（{date}）。復元データを読み込むためにアプリを再起動します。":
    "Verified {tables} tables from {date}. The app restarts to load the restored data.",
  "OAuth でサインイン": "Sign in with OAuth",

  // Composer / transcript landmarks
  "メッセージを入力": "Write a message",
  "会話": "Conversation",

  // First-run wizard. The rail labels are translated at render from a table in
  // `Onboarding.tsx`, so they never appear as a `t()` literal in a test sweep.
  "セットアップの進行状況": "Setup progress",
  "モデル提供元": "Model provider",
  "作業フォルダ": "Working folder",
  "あとで設定する": "Set up later",
  "設定はあとから変更できます": "You can change all of this later",
  "始める": "Get started",
  "ようこそ": "Welcome",
  "表示言語": "Display language",
  "ローカルで動く AI ワークスペースです。": "A local AI workspace that runs on your own machine.",
  "設定はあとからいつでも変更できます。": "You can change this at any time, from Settings.",
  "質問、対談、アイデア出し": "Questions, discussion, brainstorming",
  "思考モード": "Thinking mode",
  "長い推論の過程をテーブルで": "Shows the steps of a long chain of thought",
  "フォルダの中のファイルを読み書き": "Reads and writes files inside a folder",
  "ブラウザ操作": "Browser control",
  "実際にブラウザを操作して検証": "Drives a real browser to verify results",

  "モデル提供元の種類": "Kind of model provider",
  "この PC で動かす": "Runs on this PC",
  "クラウド API": "Cloud API",
  "API キーは不要です。この端末の上だけで動きます。": "No API key. Everything stays on this machine.",
  "LM Studio や Ollama などが起動中か調べます。": "Looks for a running LM Studio, Ollama, or similar.",
  "見つかりませんでした。LM Studio か Ollama を起動してからもう一度検出するか、「クラウド API」タブを使ってください。":
    "Nothing was found. Start LM Studio or Ollama and scan again, or use the Cloud API tab.",
  "API キーは OS の資格情報ストレージに保存され、設定画面には表示されません。":
    "The API key is stored in your OS credential store and is never shown again in Settings.",
  "API キー": "API key",
  "このプロバイダーでは API キーが必要です": "This provider requires an API key",
  "このプロバイダーでは API キーは任意です": "This provider does not need an API key",
  "選択してください…": "Choose one…",
  "プリセットを選ぶと入力されます": "Filled in from the preset",
  "確認中…": "Checking…",
  "接続してモデル一覧を取得": "Connect and fetch models",

  "フォルダを選択してください": "Choose a folder first",
  "MIX-agent がファイルを読む・書く場所です。プロジェクトを作ると、そのフォルダの中だけが対象になります。":
    "This is where MIX-agent reads and writes files. Creating a project limits it to the contents of that folder.",
  "既存のフォルダを選んでください（新しいフォルダは作成されません）":
    "Pick a folder that already exists — this wizard does not create one.",
  "このプロジェクトを使う": "Use this project",
  "このフォルダを使う": "Use this folder",
  "あとから設定する": "Decide later",

  "準備ができました": "You're all set",
  "チャットを始められます。ファイルの読み書きやシェル実行は、実行のたびに確認が出ます。":
    "You can start chatting. Reading or writing files, and running shell commands, always ask first.",
  "件": "models",
  "未設定（チャットはプロジェクトなしで始まります）":
    "Not set (chats will start without a project)",
  "チャットを始める": "Start chatting",
  "設定はここで変えられます": "You can change any of this from Settings",

  // Status indicators. These used to be bare `✓` glyphs and coloured dots with
  // no text at all, so the state was unreachable without seeing the colour;
  // see components/ui/status.tsx.
  "対応": "Yes",

  // Confirmations. These used to be `window.confirm` strings; the dialog
  // primitive splits them into a title and a consequence, so the question and
  // the "what you lose" line are separate keys.
  "スレッドを削除しますか？": "Delete this thread?",
  "プロバイダーを削除しますか？": "Delete this provider?",
  "このプロバイダーのモデル一覧も消えます。": "Its model list goes with it.",
  "MCP 接続を削除しますか？": "Delete this MCP connection?",
  "Memory を削除しますか？": "Delete this memory?",
  "ドキュメントを削除しますか？": "Delete this document?",
  "プロジェクトを削除しますか？": "Delete this project?",
  "会話は残ります。": "Its chats are kept.",
  "資料を削除しますか？": "Remove this reference?",
  "この版に戻しますか？": "Restore this revision?",
  "{at} の内容で上書きされます。": "The current text is overwritten with the version from {at}.",

  // Schedules — this pane was almost entirely untranslated, including the
  // enable/disable toggle's accessible name, which was the literal "Stop".
  "Cron で定期実行。アプリ起動中 (および次回起動時に 1 回) 自動実行されます。":
    "Runs on a cron schedule, while the app is open and once more at next launch.",
  "定期ジョブを追加": "Add a scheduled job",
  "登録済みジョブ": "Scheduled jobs",
  "ジョブはまだありません。": "No scheduled jobs yet.",
  "ジョブを削除しますか？": "Delete this job?",
  "Job 名": "Job name",
  "Cron 式": "Cron expression",
  "Cron 式 (例: 0 9 * * *)": "Cron expression (e.g. 0 9 * * *)",
  "実行プロンプト…": "Prompt to run…",
  "モデル: 会話の既定": "Model: the chat's default",
  "次回": "Next",
  "最終実行": "Last run",
  "停止中": "Paused",
  "有効にする": "Enable",
  "登録": "Add",

  // Skills — likewise untranslated.
  "取り込み元": "Import from",
  "SKILL.md を貼る": "Paste SKILL.md",
  "zip ファイル": "zip file",
  "ディレクトリ": "Directory",
  ".skill.zip を選択": "Choose a .skill.zip",
  "選択": "Choose",
  "まだ Skill はありません。": "No skills yet.",
  "resources": "resources",
  "{name} を {target} に書き出しました": "Exported {name} to {target}",
  "同梱リソースを読み込む": "Load bundled resources",
  "同梱リソースはありません。": "No bundled resources.",

  // Browser
  "プロファイルをリセットしますか？": "Reset this profile?",
  "Cookie・localStorage・キャッシュが消えます。": "Its cookies, localStorage and cache are cleared.",
  "リセット": "Reset",
  "実行": "Run",

  // Local runtime scan
  "検出中…": "Scanning…",
  "ランタイム": "Runtime",
  "オンライン": "Online",
  "{hosts} を {ms}ms で走査 — {found}": "Swept {hosts} in {ms}ms — {found}",
  "{n} 件が起動中": "{n} running",
  "起動中のランタイムは見つかりませんでした": "No running runtimes found",
  "{port} 番: {note}": "Port {port}: {note}",
  "{port} 番は応答していますが、LLM サーバーではありません":
    "Port {port} answered but is not an LLM server",
  "{label} を追加しました（{n} モデル）": "Added {label} ({n} models)",
  "{label} / {n} モデル — 上のテーブルから追加できます":
    "{label} / {n} models — add it from the table above",
  "Skill を削除しますか？": "Delete this skill?",

  // MCP
  "MCP サーバはまだありません": "No MCP servers yet",
  "接続を追加して「すべて接続」を押すと agent が使える tool として公開されます。":
    "Add a connection and press Connect all to publish its tools to the agent.",
  "すべて接続": "Connect all",
  "再接続": "Reconnect",
  "接続中…": "Connecting…",
  "接続済み": "Connected",
  "未接続": "Not connected",
  "ツール数": "Tools",
  "状態": "Status",
  "MCP サーバを追加": "Add an MCP server",
  "起動コマンド (例: npx -y @modelcontextprotocol/server-filesystem /workspace)":
    "Launch command (e.g. npx -y @modelcontextprotocol/server-filesystem /workspace)",
  "登録済みサーバ": "Registered servers",
  "{count} 個の MCP tool を agent から使えます。": "{count} MCP tools are now available to the agent.",
  "ブラウザでサインインを完了してください。アプリがリダイレクトを受け取ります。":
    "Finish signing in in the browser; the app picks up the redirect.",

  // Providers / models
  "プリセットを選んで API キーを入力してください。": "Pick a preset and enter an API key.",
  "プリセット": "Preset",
  "プリセットを選択…": "Choose a preset…",
  "表示名": "Display name",
  "種類": "Kind",
  "保存済み": "Saved",
  "能力を確認": "Verify",

  // Memory
  "検証済み": "Verified",
  "{n} モデル": "{n} models",

  "セットアップガイド": "Setup guide",

  // Settings — setup guide
  "もう一度見る": "Show me again",
  "モデル提供元の追加方法と、作業フォルダの選び方を確認できます。":
    "Walk through adding a model provider and choosing a working folder.",
};

/** The Japanese-key dictionary itself, exported for tests and tooling.
 *
 *  The empty-chat greeting is no longer keyed here: it lives in
 *  `features/chat/greeting/translations.ts`, keyed by stable fragment id, so
 *  its Japanese wording can change without breaking the English text. */
export const dictionary = en;

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
