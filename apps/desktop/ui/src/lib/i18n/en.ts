/// English translations keyed by the canonical Japanese UI string.
///
/// Split out of `lib/i18n.ts` so the table lives in its own module:
/// translators edit copy here without touching lookup logic, and new code
/// that needs stable identifiers should prefer the id-keyed pattern used
/// by the chat greeting (`features/chat/greeting/translations.ts`,
/// `greetingText(id, fallback)`) instead of adding Japanese-literal keys.
///
/// In `ja` mode `t()` returns the key itself, so this table is only
/// consulted in `en` mode — there is no per-locale lazy load to do: the
/// switch is a synchronous pure function over one variable.
///
/// Formatting note: keys stay quoted (see `.prettierrc` `quoteProps`) so the
/// static key-coverage test can parse this table.

export const en: Record<string, string> = {
  // Shell / sidebar
  "新規スレッド": "New thread",
  "新規チャット": "New chat",
  "新規タスク": "New task",
  "スレッドを作成できませんでした": "Could not create the thread",
  "タスクを作成できませんでした": "Could not create the task",
  "チャットを検索": "Search chats",
  "タスクを検索": "Search tasks",
  "チャット履歴がありません": "No chat history yet",
  "タスクがありません": "No tasks yet",
  "一致するチャットがありません": "No chats match your search",
  "一致するタスクがありません": "No tasks match your search",
  "新しい会話を始めましょう": "Start a new conversation",
  "新しいタスクを始めましょう": "Start a new task",
  "プロジェクト未選択": "No project selected",
  "プロジェクトを変更": "Change project",
  "タスク": "Task",
  "ピン留め済み": "Pinned",
  "ピン留めを解除": "Unpin",
  "今日": "Today",
  "昨日": "Yesterday",
  "過去7日": "Previous 7 days",
  "それより前": "Older",
  "ワークスペース": "Workspace",
  "設定": "Configure",
  "ナレッジ": "Knowledge",
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
  "エージェントにタスクを指示… (ターミナル実行・ファイル編集が可能)":
    "Instruct the agent… (terminal execution & file edits allowed)",
  "エージェントワークスペース": "Agent Workspace",
  "デスクトップエージェント": "Desktop Agent",
  "プロジェクトを選択": "Select a project",
  "送信": "Send",
  "停止": "Stop",
  "画像を添付": "Attach image",
  "音声入力": "Voice input",
  "このブラウザは音声入力に対応していません":
    "This browser does not support voice input",
  "モデル未設定": "No model selected",
  "Auto": "Auto",
  "リクエストごとに最適なモデルを選択": "Pick the best model for each request",
  "Auto候補にする": "Include in Auto candidates",
  "Auto候補: {n} 件": "Auto candidates: {n}",
  "選択したモデルが利用できません": "Selected model is unavailable",
  "選択したモデルのプロバイダーが削除されています。モデルを再選択してください。":
    "The provider for the selected model has been deleted. Please pick another model.",
  "画像を添付するにはVision対応モデルを選択してください":
    "Select a vision-capable model to attach images",
  "このモデルは画像入力に対応していません":
    "This model does not support image input",
  "添付ファイルが大きすぎます (上限8MB)":
    "Attachment is too large (8 MB limit)",
  "モード": "Mode",
  "改行": "newline",
  "文字": "chars",
  "プロジェクトがありません": "No projects yet",
  "クリア": "Clear",
  "プロジェクト紐付けあり": "Linked to a project",
  "サイドバーの表示切り替え": "Toggle sidebar",
  "サイドバー切り替え": "Toggle sidebar",
  "ファイルツリー表示切替": "Toggle file tree",
  "設定を開く": "Open settings",

  // Transcript
  "何でも質問してください。/ でコマンド、⌘K で検索。":
    "Ask anything. / for commands, ⌘K to search.",
  "思考プロセス": "Reasoning",
  "思考プロセスをコピー": "Copy reasoning",
  "思考中…": "Thinking…",
  "実行中…": "Running…",
  "件のツール": "tools",
  "ステップ": "steps",
  "差分": "Diff",
  "コピー": "Copy",
  "コピーしました": "Copied",
  "コピー済み": "Copied",
  "コピーできませんでした": "Copy failed",
  "出力をコピー": "Copy output",
  "エラー内容をコピー": "Copy error message",
  "詳細を表示": "Show details",
  "詳細を折りたたむ": "Hide details",
  "実行コマンド": "Command",
  "引数の詳細 (JSON)": "Argument details (JSON)",
  "（出力なし）": "(No output)",
  "モデル提供元が接続されていません": "No model providers connected",
  "会話を始めるには、Ollamaなどのローカルモデルを検出するか、APIキーを設定してください。":
    "To start a conversation, detect local models like Ollama or configure an API key.",
  "初期設定ウィザードを開く": "Open setup wizard",
  "引数": "Arguments",
  "結果": "Result",
  "最新へ": "Jump to latest",
  "応答が完了しました": "Response complete",
  "チェックポイント": "Checkpoints",
  "再開": "Resume",
  "計画": "Plan",
  "変更の確認中": "Verifying a change",
  "検証に失敗しました": "Verification failed",
  "実行に失敗しました": "The run failed",

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
  "破壊的ツール {tool} が実行されました。確認の読み取りが期待されます。":
    "Destructive tool {tool} fired; a confirming read is expected.",
  "ステップ上限 {steps} / ツール上限 {calls} に到達しました。":
    "Reached the step limit ({steps}) / tool limit ({calls}).",

  // Tool activity
  "このRunのアクティビティ": "Run activity",
  "アクティビティ": "Activity",
  "アクティビティはありません": "No activity",
  "実行中": "Running",
  "成功": "Success",
  "失敗": "Failed",
  "新規ファイル": "New file",
  "閉じる": "Close",
  "最小化": "Minimize",
  "最大化": "Maximize",
  "元のサイズに戻す": "Restore",
  "ウィンドウ操作": "Window controls",

  // Generic
  "保存": "Save",
  "キャンセル": "Cancel",
  "編集": "Edit",
  "メッセージを編集": "Edit message",
  "⌘+Enter で保存": "⌘+Enter to save",
  "作成": "Create",
  "検索": "Search",
  "読み込み中…": "Loading…",
  "エラー": "Error",
  "初期設定": "Initial setup",
  "会話から自動でメモリを作成":
    "Create memories automatically from conversations",
  "応答のあとに、事実・好み・制約だけを抽出して保存します。「覚えて」「忘れて」の指示は常に即時処理されます。":
    "After each reply, only durable facts, preferences and constraints are extracted and saved. Explicit remember / forget instructions are always handled immediately.",
  "プロバイダーを追加": "Add provider",
  "ローカル LLM を検出": "Detect local LLMs",
  "LM Studio や Ollama などのローカル推論サーバーを自動で探します。":
    "Automatically look for local inference servers such as LM Studio or Ollama.",
  "この PC で検出できるランタイムはありません。下の手動指定か、上のプリセットから追加してください。":
    "No runtimes were detected on this machine. Use the manual field below, or add a preset above.",
  "Swagger や任意のポートで起動中のサーバーも指定できます。例: http://192.168.1.10:11434/v1":
    "Also accepts a server on a custom port or a network address. e.g. http://192.168.1.10:11434/v1",
  "取り込む": "Import",
  "取り込み済み": "Imported",
  "プロバイダーとして追加してモデルを登録":
    "Register as a provider and add its models",
  "モデルが 1 つもロードされていません": "No models are loaded",
  "起動してください": "Start the server",
  "未起動": "Not running",
  "未設定": "Not set",
  "保存しました": "Saved",
  "無効": "Disabled",
  "有効": "Enabled",
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
  "未追跡ファイルは差分に含まれません":
    "Untracked files are not part of a diff",
  "git リポジトリではありません": "Not a git repository",
  "作業ツリー": "Working tree",
  "ステージ済み": "Staged",
  "未追跡": "Untracked",
  "追加": "Added",
  "改名": "Renamed",
  "バイナリファイル": "Binary file",
  "ファイルが大きすぎるため冒頭のみ表示します":
    "File too large; showing the beginning only",
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
  "フォルダ": "Folder",
  "このフォルダがエージェントの作業ディレクトリになります":
    "This folder becomes the agent's working directory",
  "を検出すると自動で読み込みます": "is picked up automatically when present",
  "プロジェクトへの指示…": "Instructions for this project…",
  "資料テキストを貼り付け…": "Paste reference text…",
  "プロジェクト単位の資料と指示を管理します。":
    "Group chats, instructions and reference texts by project.",
  "（空）": "(empty)",

  // Settings pane tabs / groups
  "アプリ": "App",
  "構成": "Configuration",
  "外観": "Appearance",
  "一般": "General",
  "メッセージ送信キー": "Message send key",
  "チャット入力欄でのメッセージ送信ショートカットを設定します。":
    "Configure the keyboard shortcut used to send messages in the chat composer.",
  "Enter で送信 (Shift+Enter で改行)":
    "Send with Enter (Shift+Enter for newline)",
  "⌘ / Ctrl + Enter で送信 (Enter で改行)":
    "Send with ⌘ / Ctrl + Enter (Enter for newline)",
  "標準幅に戻す": "Reset panel width",
  "パネル幅を拡大": "Expand panel width",
  "ネットワーク": "Network",
  "アップデート": "Updates",
  "アップデート状態": "Update status",
  "最終確認": "Last checked",
  "GitHub Releases 経由で最新リリースを確認します":
    "Checks for latest releases via GitHub Releases",
  "確認中...": "Checking...",
  "今すぐ更新を確認": "Check for updates",
  "お使いのバージョンは最新です": "You are on the latest version",
  "更新の確認に失敗しました": "Failed to check for updates",
  "利用可能なアップデート": "Update available",
  "ダウンロードして再起動": "Download and restart",
  "アップデートをダウンロード中...": "Downloading update...",
  "ダウンロード中...": "Downloading...",
  "リリースノート": "Release notes",
  "アップデート設定": "Update settings",
  "起動時にバックグラウンドで新しいバージョンを確認します":
    "Check for new versions in the background on startup",
  "アプリの改善のため匿名の診断情報を共有します":
    "Share anonymous diagnostics to improve the app",
  "バックアップ": "Backup",
  "設定セクションを選択": "Select a settings section",

  // Surfaces (top-level switcher)
  "作成、学習、探索": "Create, learn, explore",
  "ビルド、デバッグ、リリース": "Build, debug, release",
  "サーフェスを切り替え": "Switch surface",

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
  "キーワード (例: rust, deploy, postgres)":
    "Keyword (e.g. rust, deploy, postgres)",
  "登録済み Memory": "Saved memories",
  "まだ Memory がありません。": "No memories yet.",
  "検証待ち": "Pending verification",
  "検証で矛盾": "Verification disputed",
  "検証却下": "Verification rejected",

  // Memory pane — §1's columns and §3's transitions made visible.
  "{n} リンク": "{n} links",
  "リンクを表示": "Show links",
  "{name} を開く": "Open {name}",
  "置換した Memory": "Replaced by",
  "矛盾する Memory": "Contradicted by",
  "クエリに一致せず、リンクから展開された行（§4）":
    "Not matched by the query — pulled in over an edge (§4)",

  // Memory pane — §3 `resolve_conflict`.
  "未解決の矛盾": "Unresolved conflict",
  "相手: {name}": "Other side: {name}",
  "矛盾を解決しますか？": "Resolve this conflict?",
  "解決": "Resolve",
  "既定どおり解決": "Resolve as default",
  "既定: 保存済み側が勝つ（§3）": "Default: the already-stored side wins (§3)",
  "この Memory を採用": "Adopt this memory",
  "この Memory を却下": "Reject this memory",
  "相手の「{name}」を採用し、この Memory は superseded になります。":
    "Adopts “{name}” and marks this memory superseded.",
  "この Memory を採用し、相手の「{name}」は superseded になります。":
    "Adopts this memory and marks “{name}” superseded.",
  "相手の「{name}」を採用し、この Memory は expired になります。":
    "Adopts “{name}” and marks this memory expired.",
  "{winner} を採用し、{loser} は {state} になりました。":
    "{winner} adopted; {loser} is now {state}.",
  "どちらも行は削除されず、両側に改訂が残ります（§3）。":
    "Neither row is deleted and both sides keep a revision (§3).",

  // Memory pane — §4's associations screen. `kind` itself is never translated:
  // it is §2's own vocabulary and a gloss would be a second name for it.
  "無題の Memory": "Untitled memory",
  "この Memory": "this memory",
  "§4 の向き: target は列を持つ行、source はその列が指す Memory。":
    "§4 direction: target is the row carrying the column, source is the Memory it names.",
  "リンクはまだありません。": "No links yet.",
  "想起のたびに関連する Memory と結び、上書き・矛盾の鏡は §3 の遷移が書き込みます。":
    "Recall links related memories; the supersede and contradict mirrors are written by §3's transitions.",
  "この Memory が終点（target）— 列を持つ行":
    "This memory is the target — the row carrying the column",
  "この行の §3 の列が相手を名指しています。":
    "This row's §3 column names the other side.",
  "この Memory が始点（source）— 列が指す側":
    "This memory is the source — the side the column points at",
  "相手の行の §3 の列がこの Memory を名指しています。":
    "The other row's §3 column names this memory.",
  "{n} 件": "{n} items",
  "関係の種類（§2 relation_kinds）": "Relation kind (§2 relation_kinds)",
  "{name} のリンクを表示": "Show links for {name}",

  // Tools pane
  "まだ登録されていません。": "Nothing registered yet.",
  "説明": "Description",
  "既定権限": "Default permission",
  "権限ルール": "Permission rules",
  "スコープ付きルール": "Scoped rules",
  "読み取りは許可、書き込み・削除は承認":
    "Reads are allowed; writes and deletes ask for approval",

  // Approvals pane
  "承認待ち": "Approval required",
  "詳細を確認": "Check details",
  "承認待ちのアクションはありません": "No actions are waiting for approval",
  "チャット内でもインラインで承認できます。":
    "You can also approve inline from the chat.",

  // Agents pane
  "ツールを絞った専用エージェントを定義します。":
    "Define dedicated agents restricted to a subset of tools.",
  "Agent 名": "Agent name",
  "System instructions…": "System instructions…",

  // Projects pane
  "Project 名": "Project name",
  "まだありません。": "Nothing here yet.",

  // Models pane
  "ラベル": "Label",
  "Vision": "Vision",

  // Model picker (chat composer / settings)
  "モデルを検索": "Search models",
  "モデルを選択": "Select a model",
  "お気に入り": "Favourites",
  "最近使ったモデル": "Recently used",
  "モデルがありません": "No models yet",
  "プロバイダーを追加してモデルを取得してください":
    "Add a provider and fetch its models",
  "該当するモデルがありません": "No models match",
  "お気に入りに追加": "Add to favourites",
  "お気に入りから外す": "Remove from favourites",
  "モデルを管理": "Manage models",
  "プロバイダーで絞り込む": "Filter by provider",
  "すべてのプロバイダー": "All providers",
  "{shown} / {total} 件のモデル": "{shown} / {total} models",
  "モデルの対応機能は確認結果をもとに表示されます。誤りがあれば再確認してください。":
    "Model capabilities reflect the last check. Re-verify if anything looks wrong.",
  "一時モード": "Temporary mode",
  "この送信ではMemoryを使いません。Run終了後に会話・回答・送信した添付を削除します。":
    "This send does not use Memory. The chat, answer and any attachments are deleted when the run ends.",
  "Toolを許可": "Allow tools",
  "Toolの外部送信や副作用は残る場合があります。":
    "Tools may still send data out or leave side effects.",
  "一時モード: この送信は保存されません":
    "Temporary mode: this send is not saved",

  // Diff preview
  "大きな差分のため先頭2000行のみ表示します":
    "Showing the first 2000 lines of a large diff",
  "差分: {n}": "Diff: {n}",
  "すべて表示 ({n})": "Show all ({n})",

  // Shared components / error boundary
  "インターフェースの起動に失敗しました": "The interface failed to start",
  "再読み込み": "Reload",

  // Diagnostics pane
  "OK": "OK",
  "Database at-rest encryption is not enabled in this build. Conversation logs, run history and tool arguments are stored in plaintext; rely on disk-level encryption (FileVault / BitLocker / LUKS) until the SQLCipher migration lands.":
    "Database at-rest encryption is not enabled in this build. Conversation logs, run history and tool arguments are stored in plaintext; rely on disk-level encryption (FileVault / BitLocker / LUKS) until the SQLCipher migration lands.",

  // Browser pane (English-canonical keys live alongside Japanese ones here;
  // both spellings stay registered while call sites migrate)
  "Chromiumが見つかりません。システムのChromiumを入れるか、バンドル版を用意してください。":
    "No Chromium binary resolved. Install a system Chromium or bundle one to enable the Browser.",

  // Health indicator / update banner
  "正常": "OK",
  "接続エラー": "Connection error",
  "確認中": "Checking…",
  "ヘルス": "Health",
  "アップデートがあります": "Update available",
  "インストールして再起動": "Install & restart",

  // Command palette
  "一致する項目がありません": "No matching commands",
  "別のキーワードで検索してください": "Try a different keyword",
  "開く": "Open",
  "アクション": "Actions",
  "サーフェス": "Surfaces",
  "表示": "Views",
  "スレッド": "Threads",

  // Keyboard shortcut cheat sheet
  "キーボードショートカット": "Keyboard shortcuts",
  "ショートカット一覧": "Show keyboard shortcuts",
  "移動": "Navigate",
  "入力": "Compose",
  "チャットサーフェスへ": "Go to the chat surface",
  "エージェントサーフェスへ": "Go to the agent surface",
  "この一覧を表示": "Show this list",
  "ダイアログを閉じる": "Close the dialog",

  // Settings: quick prompts, network, updates, mode select
  "Egress 許可リスト": "Egress allowlist",
  "未設定時は Tool がすべての公開ドメインへ到達可能。ワイルドカード (*) と完全一致ドメインを混在可。":
    "With no list, tools can reach any public domain. Wildcard (*) and exact hosts can be mixed.",
  "自動更新を有効化": "Enable automatic updates",
  "匿名の利用統計を共有 (将来)":
    "Share anonymous usage statistics (coming soon)",

  // Backup / restore, MCP OAuth, skills export, sidebar landmark
  "サイドバー": "Sidebar",
  "サイドパネル": "Side panel",
  "パネル切り替え": "Switch panel",
  "表": "Table",
  "メインコンテンツへ移動": "Skip to main content",
  "絞り込みをクリア": "Clear filters",
  "システム": "System",
  "バックアップ / 復元": "Backup / restore",
  "データベース全体をスナップショットし、圧縮・暗号化します (Argon2id + AES-256-GCM)。API キーは OS キーチェーンに残るため含まれません。":
    "The whole database is snapshotted, compressed and encrypted (Argon2id + AES-256-GCM). API keys stay in the OS Keychain and are not included.",
  "バックアップパスワード": "Backup password",
  "保存先パス": "Destination path",
  "暗号化バックアップを作成": "Create an encrypted backup",
  "復元する既存の .mixbak": "Existing .mixbak to restore",
  "ファイルを選択": "Choose a file",
  "参照": "Browse",
  "URL": "URL",
  "待機条件": "Wait condition",
  "JavaScript 式": "JavaScript expression",
  "実行する JavaScript": "JavaScript to run",
  "バックアップを検証し、再起動時に復元":
    "Verify a backup and reload it on restart",
  "復元": "Restore",
  "今すぐ再起動": "Restart now",
  "{bytes} バイト（{tables} テーブル）を {path} に保存しました":
    "Saved {bytes} bytes ({tables} tables) to {path}",
  "{tables} テーブルを検証しました（{date}）。復元データを読み込むためにアプリを再起動します。":
    "Verified {tables} tables from {date}. The app restarts to load the restored data.",
  "OAuth でサインイン": "Sign in with OAuth",

  // Composer / transcript landmarks
  "メッセージを入力": "Write a message",
  "メッセージを入力してください": "Type a message first",
  "モデルを選択してください": "Select a model first",
  "実行中です": "A run is in progress",
  "会話": "Conversation",
  "貼り付け・ドロップにも対応": "Paste & drop supported",
  "添付は最大{max}件までです": "Up to {max} attachments",
  "実行中は編集できません": "Cannot edit while a run is in progress",
  "実行中は再試行できません": "Cannot retry while a run is in progress",
  "再試行": "Retry",
  "折りたたむ": "Collapse",

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
  "ローカルで動く AI ワークスペースです。":
    "A local AI workspace that runs on your own machine.",
  "設定はあとからいつでも変更できます。":
    "You can change this at any time, from Settings.",
  "質問、対談、アイデア出し": "Questions, discussion, brainstorming",
  "思考モード": "Thinking mode",
  "長い推論の過程をテーブルで": "Shows the steps of a long chain of thought",
  "フォルダの中のファイルを読み書き": "Reads and writes files inside a folder",
  "ブラウザ操作": "Browser control",
  "実際にブラウザを操作して検証": "Drives a real browser to verify results",

  "モデル提供元の種類": "Kind of model provider",
  "この PC で動かす": "Runs on this PC",
  "クラウド API": "Cloud API",
  "API キーは不要です。この端末の上だけで動きます。":
    "No API key. Everything stays on this machine.",
  "LM Studio や Ollama などが起動中か調べます。":
    "Looks for a running LM Studio, Ollama, or similar.",
  "見つかりませんでした。LM Studio か Ollama を起動してからもう一度検出するか、「クラウド API」タブを使ってください。":
    "Nothing was found. Start LM Studio or Ollama and scan again, or use the Cloud API tab.",
  "API キーは OS の資格情報ストレージに保存され、設定画面には表示されません。":
    "The API key is stored in your OS credential store and is never shown again in Settings.",
  "API キー": "API key",
  "このプロバイダーでは API キーが必要です":
    "This provider requires an API key",
  "このプロバイダーでは API キーは任意です":
    "This provider does not need an API key",
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
  "このエージェントを削除しますか？": "Delete this agent?",
  "{name} は元に戻せません。": "{name} cannot be restored.",
  "{name} を削除": "Delete {name}",
  "資料を削除しますか？": "Remove this reference?",
  "この版に戻しますか？": "Restore this revision?",
  "{at} の内容で上書きされます。":
    "The current text is overwritten with the version from {at}.",

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
  "{name}を停止": "Stop {name}",
  "{name}を有効にする": "Enable {name}",
  "登録": "Add",

  // Skills — likewise untranslated.
  "取り込み元": "Import from",
  "SKILL.md を貼る": "Paste SKILL.md",
  "zip ファイル": "zip file",
  "ディレクトリ": "Directory",
  ".skill.zip を選択": "Choose a .skill.zip",
  "選択": "Choose",
  "まだ Skill はありません。": "No skills yet.",
  "{name} を {target} に書き出しました": "Exported {name} to {target}",
  "同梱リソースを読み込む": "Load bundled resources",
  "同梱リソースはありません。": "No bundled resources.",

  // Browser
  "プロファイルをリセットしますか？": "Reset this profile?",
  "Cookie・localStorage・キャッシュが消えます。":
    "Its cookies, localStorage and cache are cleared.",
  "リセット": "Reset",
  "実行": "Run",
  "開くURL": "URL to open",

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
  "接続中…": "Connecting…",
  "接続済み": "Connected",
  "未接続": "Not connected",
  "ツール数": "Tools",
  "状態": "Status",
  "MCP サーバを追加": "Add an MCP server",
  "起動コマンド (例: npx -y @modelcontextprotocol/server-filesystem /workspace)":
    "Launch command (e.g. npx -y @modelcontextprotocol/server-filesystem /workspace)",
  "登録済みサーバ": "Registered servers",
  "{count} 個の MCP tool を agent から使えます。":
    "{count} MCP tools are now available to the agent.",
  "ブラウザでサインインを完了してください。アプリがリダイレクトを受け取ります。":
    "Finish signing in in the browser; the app picks up the redirect.",

  // Providers / models
  "プリセットを選んで API キーを入力してください。":
    "Pick a preset and enter an API key.",
  "プリセット": "Preset",
  "プリセットを選択…": "Choose a preset…",
  "表示名": "Display name",
  "保存済み": "Saved",
  "能力を確認": "Verify",
  "プロバイダーを検索": "Search providers",
  "該当するプロバイダーがありません": "No providers match",
  "プリセットの既定値を編集できます": "You can edit the preset's default",
  "空欄のままにすると既存のキーを保持します":
    "Leave blank to keep the existing key",
  "{name} を有効にする": "Enable {name}",
  "{n} 件を取り込みました": "Imported {n}",
  "接続してモデルを取得": "Connect and fetch models",

  // Memory
  "検証済み": "Verified",
  "{n} モデル": "{n} models",

  "セットアップガイド": "Setup guide",

  // Settings — setup guide
  "もう一度見る": "Show me again",
  "モデル提供元の追加方法と、作業フォルダの選び方を確認できます。":
    "Walk through adding a model provider and choosing a working folder.",
  "新しいスレッドで既定で使うモデル": "Default model for new threads",
  "類似するメモリ同士を自動でリンクする": "Automatically link similar memories",
  "抽出後に埋め込みの近傍を調べ、関連する Memory をリンクします。リンクは想起のたびに1ホップ展開されます。":
    "After extraction, looks up embedding neighbours and links the related memories. Links are expanded one hop on every recall.",
  "既定はオフです。": "Off by default.",

  // Normalised from English-literal keys (`t("Copy")`, …): the
  // canonical key is the Japanese copy, so the reverse map is gone.
  // Added only when a rewritten call site references the key.
  "有効としてマーク": "Mark as still valid",
  "編集せずに検索": "Search instead of editing",
  "改訂履歴": "Revision history",
  "まだ改訂がありません。": "No revisions yet.",
  "復元する": "restore",
  "検証する": "verify",
  "ピン留め": "Pin",
  "バックエンド": "Backend",
  "状態を更新": "Refresh status",
  "ヘッドレス": "headless",
  "ヘッドあり": "headed",
  "プロファイル": "profiles",
  "成果物": "artifacts",
  "ワークスペースのプロファイル": "Workspace profile",
  "新規プロファイル": "new profile",
  "(未作成)": "(not provisioned)",
  "操作": "Action",
  "まだ操作がありません。": "No actions yet.",
  "出力": "output",
  "既知のプロファイル": "Known profiles",
  "準備完了": "ready",
  "空": "empty",
  "各ワークスペースは独自のCookie・localStorage・キャッシュを持ち、エージェントのワークスペースと一致します。":
    "Each workspace keeps its own cookies, localStorage and cache, matching the agent's workspace.",
  "CSSセレクター（例: #search、body）": "CSS selector, e.g. #search or body",
  "入力する文字": "text to type",
  "URLに含まれるまで待機…": "wait until URL contains…",
  "縦スクロール量": "scroll dy",
  "コンテキスト": "Context",
  "推論": "Reasoning",
  "ストレージ": "Storage",
  "シークレット (Keychain)": "Secrets (Keychain)",
  "エンジン": "Engine",
  "件数": "Counts",
  "メモリー": "Memories",
  "実行中のRun": "Active Runs",
  "データベースの暗号化": "Database Encryption",
  "保存時の暗号化": "At-rest encryption",
  "ローカルデータベースの暗号化を有効にします。既定はオフ（Codex互換の平文SQLite）です。":
    "Enable encryption for the local database. Defaults to off (Codex-compatible plain SQLite).",
  "保存時は暗号化済み": "Encrypted at rest",
  "未暗号化（メモリ内 + Keychainのみ）":
    "Not encrypted (in-memory + Keychain only)",
  "暗号化済み": "Encrypted",
  "保留中": "Pending",
  "マスターキーはOSのKeychainにあります":
    "Master key is present in the OS Keychain",
  "マスターキーは初回利用時に作成されます":
    "Master key will be created on first use",
  "Keychainの状態は不明": "Keychain status unknown",
  "システム指示": "System instructions",
  "リソース": "resources",
  "バイナリまたは読み取れない旧内容": "binary or unreadable old content",
  // Placeholder-bearing metric labels (Memory / Associations panes call these
  // via `tf`, which the literal sweep counts separately from `t`).
  "confidence {n}": "confidence {n}",
  "salience {n}": "salience {n}",
  "strength {n}": "strength {n}",
  "score {n}": "score {n}",
  "weight {w}": "weight {w}",
  "coactivation {n}": "coactivation {n}",
  "完了": "Completed",
  "実行されたツール": "Executed tools",
  "ファイルメンション": "File mention",
  "ファイルをメンション (@)": "Mention file (@)",
  "一致するファイルがありません": "No matching files",
  "クイックコマンド": "Quick commands",
  "クイックコマンド (/)": "Quick commands (/)",
  "テストを実行しエラーを自己修復": "Run tests and self-repair errors",
  "バグや問題点を特定して修正": "Identify and fix bugs or issues",
  "変更差分をレビューして改善点を指摘": "Review diff and suggest improvements",
  "変更内容に応じたコミットを作成": "Create a commit matching changes",
  "実装計画と手順をステップ別に策定": "Draft step-by-step implementation plan",
  "コードの仕様と動作をわかりやすく解説": "Explain code logic and architecture",
  "統合表示": "Unified diff",
  "2画面並列表示": "Split diff",
  "ステージ解除": "Unstage",
  "ステージに追加": "Stage",
  "変更を元に戻す": "Discard changes",
  "変更を破棄": "Discard changes",
  "「{path}」の変更を元に戻しますか？": 'Discard changes in "{path}"?',
  "差分パネルで開く": "Open in changes panel",
  "差分パネル": "Changes panel",
  "破棄": "Discard",
  "チャットに入力 (@{path})": "Add to chat (@{path})",
  "チャットに入力": "Add to chat",

  // Diagnostics and Safety
  "システム診断": "System Diagnostics",
  "最近のログ": "Recent logs",
  "ログを隠す": "Hide logs",
  "診断をコピー": "Copy diagnostics",
  "コピー完了": "Copied",
  "書き出し": "Export",
  "最近の実行ログ": "Recent execution logs",
  "ログ読み込み中...": "Loading logs...",
  "ログはありません": "No logs recorded",
  "ローカル実行と安全設計": "Local Execution & Safety First",
  "会話履歴や設定はあなたの端末ローカルに安全に保存されます。ファイルの編集やシェル実行などの操作は、必ず事前に承認を求める安全設計になっています。":
    "Your conversations and settings are stored safely on your local device. Operations such as file edits and shell executions are designed to require your approval beforehand.",

  // UI Polish & Docking
  "右側に固定表示": "Dock to side",
  "オーバーレイ表示に切り替え": "Switch to overlay",
  "最新のメッセージへスクロール": "Scroll to latest message",
  "生成中… 最新へ": "Generating… Jump to latest",
  "ファイルをドロップして添付": "Drop files to attach",
};
