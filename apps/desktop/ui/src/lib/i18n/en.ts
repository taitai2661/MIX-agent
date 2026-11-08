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
  "チャット履歴がありません": "No chat history yet",
  "一致するチャットがありません": "No chats match your search",
  "新しい会話を始めましょう": "Start a new conversation",
  "プロジェクト未選択": "No project selected",
  "プロジェクトを変更": "Change project",
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
  "アクセントカラー": "Accent color",
  "アクセントカラーを選択": "Choose an accent color",
  "ボタンやリンクなど、操作の目印となる色を選びます。":
    "Choose the color that marks actions such as buttons and links.",
  "緑": "Green",
  "青": "Blue",
  "藍": "Indigo",
  "紫": "Violet",
  "ピンク": "Pink",
  "オレンジ": "Orange",
  "ティール": "Teal",
  "コマンドパレット": "Command palette",

  // Chat composer
  "MIX-agent に話しかける…": "Message MIX-agent…",
  "深く考えてほしいことを入力…": "Ask for a deep, careful answer…",
  "計画したいタスクを入力…（読み取り専用）":
    "Describe what to plan… (read-only)",
  "エージェントにタスクを指示… (ターミナル実行・ファイル編集が可能)":
    "Instruct the agent… (terminal execution & file edits allowed)",
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
  "何でも質問してください。/ でコマンド、{shortcut} で検索。":
    "Ask anything. / for commands, {shortcut} to search.",
  "思考プロセス": "Reasoning",
  "思考プロセスをコピー": "Copy reasoning",
  "思考中…": "Thinking…",
  "実行中…": "Running…",
  "考え中…": "Thinking…",
  "応答を生成中…": "Writing response…",
  "待機中…": "Queued…",
  "{tool} を実行中…": "Running {tool}…",
  "このRunの経過時間": "Time elapsed for this run",
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

  // Usage
  "使用状況": "Usage",
  "このスレッドにはまだ使用状況がありません": "No usage in this thread yet",
  "メッセージを送ると、トークン数と推定コストが表示されます。":
    "Send a message to see token counts and estimated cost.",
  "最新ターンで使用したプロンプト量です。":
    "Prompt tokens used by the latest turn.",
  "このRun": "This run",
  "会話合計": "Conversation total",
  "入力トークン": "Input tokens",
  "出力トークン": "Output tokens",
  "キャッシュ読取": "Cache read",
  "キャッシュ書込": "Cache write",
  "推定コスト": "Estimated cost",
  "合計トークン": "Total tokens",
  "キャッシュヒット率": "Cache hit rate",
  "{n} Run": "{n} runs",
  "モデル詳細": "Model details",
  "コンテキストウィンドウ": "Context window",
  "ファミリー": "Family",
  "パラメータ数": "Parameters",
  "量子化": "Quantization",
  "サイズ": "Size",
  "非対応": "No",
  "Runの詳細": "Run details",
  "ステップ数": "Steps",
  "ツール呼び出し": "Tool calls",
  "失敗したツール呼び出し": "Failed tool calls",
  "トークン上限": "Token limit",
  "所要時間": "Duration",
  "開始": "Started",
  "Run ID": "Run ID",
  "待機中": "Queued",
  "予算延長待ち": "Awaiting budget",
  "中断済み": "Interrupted",
  "Run履歴": "Run history",
  "{n} ステップ": "{n} steps",
  "{n} ツール": "{n} tools",
  "残り": "Remaining",
  "残りトークン": "Tokens left",
  "時間上限": "Time limit",
  "予算延長": "Budget extensions",
  "平均コスト": "Average cost",
  "速度": "Speed",
  "平均出力速度": "Average output speed",
  "平均合計速度": "Average total speed",
  "1ステップあたり": "Per step",
  "1ツール呼び出しあたり": "Per tool call",
  "コスト/分": "Cost / min",
  "ツール実行や待ち時間を含む、Run全体の平均です。":
    "Averages across the whole run, including tool calls and waits.",
  "種別": "Kind",
  "接続先": "Endpoint",
  "対象": "Eligible",
  "対象外": "Not eligible",
  "Auto候補": "Auto candidate",

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
  "Ctrl+Enter で保存": "Ctrl+Enter to save",
  "作成": "Create",
  "検索": "Search",
  "読み込み中…": "Loading…",
  "以前のメッセージを読み込む": "Load earlier messages",
  "イベント数が上限を超えたため、この実行のログの一部は省略されています。":
    "Some run events were omitted because this run exceeded the replay limit.",
  "イベント数が上限を超えたため、この実行のログの一部は省略されています（最新の結果は表示されています）。":
    "Some run events were omitted because this run exceeded the replay limit (the latest result is shown).",
  "{count}件のイベントを省略しました": "{count} events omitted",
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
  "未起動": "Not running",
  "未設定": "Not set",
  "保存しました": "Saved",
  "未保存の変更": "Unsaved changes",
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
  "外観": "Appearance",
  "一般": "General",
  "メッセージ送信キー": "Message send key",
  "チャット入力欄でのメッセージ送信ショートカットを設定します。":
    "Configure the keyboard shortcut used to send messages in the chat composer.",
  "Enter で送信 (Shift+Enter で改行)":
    "Send with Enter (Shift+Enter for newline)",
  "{shortcut}+Enter で送信 (Enter で改行)":
    "Send with {shortcut}+Enter (Enter for newline)",
  "標準幅に戻す": "Reset panel width",
  "パネル幅を拡大": "Expand panel width",
  "サイドバーの幅を変更": "Resize sidebar",
  "ファイルツリーの幅を変更": "Resize file tree",
  "サイドパネルの幅を変更": "Resize side panel",
  "ネットワーク": "Network",
  "アップデート": "Updates",
  "アプリのアップデート": "App updates",
  "現在のバージョン: {version}": "Current version: {version}",
  "現在のバージョンを取得できませんでした":
    "Could not determine the current version",
  "更新を確認": "Check for updates",
  "ダウンロード中…": "Downloading…",
  "ダウンロード中… {percent}%": "Downloading… {percent}%",
  "署名を検証しています…": "Verifying the signature…",
  "更新をダウンロード中…": "Downloading the update…",
  "再起動": "Restart",
  "v{version} をダウンロードしました。次回 MIX-agent を終了（Quit）すると適用されます。":
    "Downloaded v{version}. It will be applied the next time you quit MIX-agent.",
  "v{version} を適用待ち": "v{version} waiting to be applied",
  "v{version} が利用可能です": "v{version} is available",
  "ダウンロード": "Download",
  "最新バージョンです。": "You are up to date.",
  "起動時に更新を確認": "Check for updates on launch",
  "起動時にGitHub Releasesのマニフェストを確認します。":
    "Checks the GitHub Releases manifest on launch.",
  "更新を自動でダウンロード": "Download updates automatically",
  "起動時に新しいバージョンを見つけたら自動でダウンロードします。適用はメニュー／トレイの Quit で終了したときに行われ、再起動は手動です。":
    "Downloads a newer version automatically when the launch check finds one. It is applied when you quit from the menu or tray; restarting is manual.",
  "Linuxのdebパッケージはアプリ内更新の対象外です。debで導入した場合はGitHub Releasesから手動で更新してください（AppImageは自動更新できます）。":
    "The Linux .deb package cannot update in-app; if you installed the .deb, update manually from GitHub Releases (AppImage builds update in-app).",
  "このビルドはアプリ内更新に対応していません。GitHub Releasesから手動で更新してください。":
    "This build cannot update in-app. Update manually from GitHub Releases.",
  "GitHub Releasesを開く": "Open GitHub Releases",
  "macOSアプリはApple Developer ID署名・公証なし（ad-hoc署名のみ）です。初回起動時にGatekeeperの確認が必要です。":
    "The macOS app is not signed with an Apple Developer ID or notarized (ad-hoc signature only). Gatekeeper asks for confirmation on first launch.",
  "Windowsインストーラーは未署名です。SmartScreenの警告が出る場合は「詳細情報」→「実行」を選択してください。":
    "The Windows installer is unsigned. If SmartScreen warns you, choose “More info” → “Run anyway”.",
  "バックアップ": "Backup",
  "設定セクションを選択": "Select a settings section",

  // Workspaces (top-level switcher)
  "作成、学習、探索。プロジェクトなしで使えます。":
    "Create, learn and explore. Works without a project.",
  "ワークスペースを切り替え": "Switch workspace",
  "コード": "Code",

  // Keys referenced from files owned elsewhere (App.tsx, LocalDiscoveryCard,
  // ProvidersPane, McpPane, …) — registered here so English never falls back
  // to the Japanese original.
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
  "読み取り専用で計画を提示": "Propose a plan without editing",

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
  "実行はバックグラウンドで継続しています。トレイから再表示できます。":
    "The run continues in the background. Reopen it from the tray.",
  "ブラウザ画面": "Browser view",
  "ブラウザ画面を開く": "Open the browser view",
  "ブラウザを表示": "Show browser",
  "ブラウザメニュー": "Browser menu",
  "50%に戻す": "Reset to 50%",
  "ブラウザセッション": "Browser session",
  "全タブを閉じる": "Close all tabs",
  "Cookie・localStorage・キャッシュが消えます。開いているタブも閉じます。":
    "Cookies, localStorage and cache will be cleared. Open tabs will close too.",
  "進む": "Forward",
  "アドレスバー": "Address bar",
  "URL または localhost:3000": "URL or localhost:3000",
  "システムのブラウザで開く": "Open in the system browser",
  "エージェント操作中": "Agent is acting",
  "エージェント使用時に自動表示": "Auto-open when the agent browses",
  "エージェント使用時の自動表示をオフ":
    "Turn off auto-open when the agent browses",
  "フル表示": "Full view",
  "スプリット表示に戻す": "Back to split view",
  "ブラウザ設定を開く": "Open browser settings",
  "スレッドを開始するとブラウザを使えます。":
    "Start a thread to use the browser.",
  "新しいタブ": "New tab",
  "タブを閉じる": "Close tab",
  "承認待ちが{count}件あります": "{count} approval(s) waiting",
  "詳細を確認": "Check details",
  "承認待ちのアクションはありません": "No actions are waiting for approval",
  "チャット内でもインラインで承認できます。":
    "You can also approve inline from the chat.",

  // Agents pane
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
  "downloading": "Downloading…",
  "verifying": "Verifying…",
  "done": "Ready",
  "system": "System",
  "none": "None",

  // Health indicator
  "正常": "OK",
  "接続エラー": "Connection error",
  "確認中": "Checking…",
  "ヘルス": "Health",

  // Command palette
  "一致する項目がありません": "No matching commands",
  "別のキーワードで検索してください": "Try a different keyword",
  "開く": "Open",
  "アクション": "Actions",
  "表示": "Views",
  "拡大": "Zoom in",
  "縮小": "Zoom out",
  "拡大率をリセット": "Reset zoom",
  "スレッド": "Threads",
  "メッセージ": "Messages",

  // Keyboard shortcut cheat sheet
  "キーボードショートカット": "Keyboard shortcuts",
  "ショートカット一覧": "Show keyboard shortcuts",
  "移動": "Navigate",
  "入力": "Compose",
  "チャットに切り替え": "Switch to Chat",
  "エージェントに切り替え": "Switch to Agent",
  "この一覧を表示": "Show this list",
  "ダイアログを閉じる": "Close the dialog",

  // Settings: quick prompts, network, updates, mode select
  "ブラウザのローカルアクセス": "Browser local access",
  "ローカル開発サーバーへのアクセスを許可": "Allow access to local dev servers",
  "localhost や社内 LAN の開発サーバーをブラウザ Tool で開けるようにします。クラウドメタデータ（169.254.169.254）は許可しても遮断されます。":
    "Lets the browser tool open localhost or LAN development servers. Cloud metadata (169.254.169.254) stays blocked even when this is on.",
  "Egress 許可リスト": "Egress allowlist",
  "未設定時は Tool がすべての公開ドメインへ到達可能。ワイルドカード (*) と完全一致ドメインを混在可。":
    "With no list, tools can reach any public domain. Wildcard (*) and exact hosts can be mixed.",

  // Backup / restore, MCP OAuth, skills export, sidebar landmark
  "サイドバー": "Sidebar",
  "サイドパネル": "Side panel",
  "パネル切り替え": "Switch panel",
  "表示パネル": "View panels",
  "基本": "Basics",
  "モデルと接続": "Models & connections",
  "データと検索": "Data & search",
  "詳細・診断": "Advanced & diagnostics",
  "UX イベント（この端末のみ）": "UX events (this device only)",
  "計測をクリア": "Clear metrics",
  "画面操作の回数をこの端末にだけ記録します。プロンプトやファイルの内容は保存しません。":
    "Records how often screens are used, on this device only. Prompts and file contents are never stored.",
  "まだ記録はありません": "No events recorded yet",
  "表": "Table",
  "メインコンテンツへ移動": "Skip to main content",
  "絞り込みをクリア": "Clear filters",
  "システム": "System",
  "バックアップ / 復元": "Backup / restore",
  "データベース全体をスナップショットし、圧縮・暗号化します (Argon2id + AES-256-GCM)。API キーは OS キーチェーンに残るため含まれません。":
    "The whole database is snapshotted, compressed and encrypted (Argon2id + AES-256-GCM). API keys stay in the OS Keychain and are not included.",
  "バックアップパスワード": "Backup password",
  "バックアップパスワード（12文字以上）": "Backup password (12+ characters)",
  "保存先パス": "Destination path",
  "暗号化バックアップを作成": "Create an encrypted backup",
  "復元する既存の .mixbak": "Existing .mixbak to restore",
  "ファイルを選択": "Choose a file",
  "参照": "Browse",
  "URL": "URL",
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
  "今はスキップ": "Skip for now",
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

  // Renewed first-run wizard (2026-11). The rail is a vertical map beside the
  // task surface, so it also carries the step's intent.
  "初回セットアップ": "First-run setup",
  "デスクトップ": "Desktop",
  "モデルを接続": "Connect a model",
  "このアプリの頭脳になるモデルを選びます。ローカルでもクラウドでもかまいません。":
    "Choose the model that powers this app — local or cloud.",
  "頭脳をつなぐ": "Connect the brain",
  "LM Studio や Ollama、またはクラウド API。あなたの環境に合うモデルを選べます。":
    "LM Studio, Ollama, or a cloud API — choose what fits your environment.",
  "作業フォルダを選ぶ": "Choose a working folder",
  "エージェントがファイルを読み書きする場所を決めます。あとから変更できます。":
    "Decide where the agent may read and write files. You can change this later.",
  "作業場所を決める": "Choose the workspace",
  "指定したフォルダの中だけが、エージェントの作業範囲になります。":
    "The agent's workspace is limited to the folder you choose.",
  "設定を確認して、最初のスレッドを開きましょう。":
    "Review your setup and open your first thread.",
  "さあ、始めましょう": "Let's get started",
  "設定はいつでも変更できます。まずは気軽に話しかけてみてください。":
    "You can change any of this later. Start by saying hello.",
  "ローカルで動く AI ワークスペース": "A local AI workspace",
  "会話もファイルも、あなたの端末の中に。プライバシーを保ったまま、日常の作業を任せられます。":
    "Conversations and files stay on your device. Keep your privacy while delegating everyday work.",
  "ローカルに保存": "Stored locally",
  "会話・ファイル・設定はこの端末の中に保存されます。":
    "Conversations, files, and settings are stored on this device.",
  "現在": "Current",
  "ステップ {n} / {total}": "Step {n} of {total}",
  "3 つのステップで、すぐに使い始められます。":
    "Three quick steps and you're ready to go.",
  "ファイルの読み書きやシェル実行は、実行のたびに確認が出ます。":
    "Reading or writing files and running shell commands asks for confirmation every time.",
  "選択したフォルダ": "Selected folder",
  "フォルダが選ばれていません": "No folder selected",

  "モデル提供元の種類": "Kind of model provider",
  "この PC で動かす": "Runs on this PC",
  "クラウド API": "Cloud API",
  "API キーは不要です。この端末の上だけで動きます。":
    "No API key. Everything stays on this machine.",
  "LM Studio や Ollama などが起動中か調べます。":
    "Looks for a running LM Studio, Ollama, or similar.",
  "見つかりませんでした。LM Studio か Ollama を起動してからもう一度検出するか、「クラウド API」タブを使ってください。":
    "Nothing was found. Start LM Studio or Ollama and scan again, or use the Cloud API tab.",
  "API キーのマスター鍵は OS の資格情報ストレージ（macOS Keychain / Windows Credential Manager）に保存され、API キー本体は AES-256-GCM で暗号化してローカルに保存されます。設定画面には表示されません。":
    "The API-key master seed is kept in the OS credential store (macOS Keychain / Windows Credential Manager), and the keys themselves are AES-256-GCM encrypted on disk. They are never shown in Settings.",
  "API キー": "API key",
  "このプロバイダーでは API キーが必要です":
    "This provider requires an API key",
  "このプロバイダーでは API キーは任意です":
    "This provider does not need an API key",
  "非公式・自己責任の機能です": "Unofficial feature — use at your own risk",
  "この連携は OpenAI の公式 API ではなく、Codex CLI が使う非公開バックエンドを利用します。利用規約・アカウント制限に抵触する可能性、予告なく停止する可能性があります。MIX-agent は動作や継続利用を保証しません。":
    "This integration uses the private backend the Codex CLI uses, not an official OpenAI API. It may violate the terms of service or account limits, and it may stop working without notice. MIX-agent does not guarantee that it will work or remain available.",
  "ChatGPT にログイン": "Sign in to ChatGPT",
  "表示された URL を開き、ワンタイムコードを入力してください。この画面は自動で完了を確認します。":
    "Open the URL and enter the one-time code. This page checks for completion automatically.",
  "コードをコピー": "Copy code",
  "認証ページを開く": "Open sign-in page",
  "認証の完了を待っています…": "Waiting for sign-in to complete…",
  "ChatGPT でログイン": "Sign in with ChatGPT",
  "ChatGPT 使用枠": "ChatGPT usage",
  "5時間枠": "5-hour window",
  "週次枠": "Weekly window",
  "使用枠": "Usage window",
  "未ログイン": "Not signed in",
  "ログイン済み": "Signed in",
  "残量を更新": "Refresh usage",
  "ログアウト": "Log out",
  "ChatGPT からログアウトしますか？": "Sign out of ChatGPT?",
  "使用枠を取得できませんでした。": "Could not read the usage windows.",
  "ChatGPT の再ログインが必要です。": "ChatGPT needs you to sign in again.",
  "ChatGPT のログインの有効期限が切れました。もう一度ログインしてください。":
    "The ChatGPT sign-in expired. Please sign in again.",
  "使用枠を取得できませんでした: {detail}": "Could not read usage: {detail}",
  "ログインの有効期限が切れました。もう一度ログインしてください。":
    "The sign-in expired. Please sign in again.",
  "ログイン情報が見つかりません。もう一度ログインしてください。":
    "The sign-in session could not be found. Please sign in again.",
  "ChatGPT のログインに失敗しました。": "ChatGPT sign-in failed.",
  "15 分以上前の情報です。": "This information is more than 15 minutes old.",
  "残量を取得しています…": "Loading usage…",
  "再ログイン": "Sign in again",
  "リセット:": "Resets:",
  "リセット時刻は端末のタイムゾーンで表示し、UTC も併記しています。":
    "Reset times are shown in your device time zone, with the UTC time alongside.",
  "非公式ChatGPT連携を有効にする": "Enable the unofficial ChatGPT integration",
  "非公式連携が無効です": "The unofficial integration is disabled",
  "このアカウントでは ChatGPT のデバイスコードログインを利用できません。":
    "Device-code sign-in is not available for this ChatGPT account.",
  "ChatGPT のログインを開始できませんでした: {detail}":
    "Could not start ChatGPT sign-in: {detail}",
  "保存した認証情報と取得済みのモデルをこの PC から削除します。再度使うにはログインし直してください。":
    "This removes the saved credentials and downloaded models from this PC. You will need to sign in again to use it.",
  "ChatGPT（非公式・専用連携）": "ChatGPT (unofficial, dedicated)",
  "OpenAI API キー方式のプロバイダーとは別の専用連携です。プロバイダー一覧には表示されず、接続したモデルだけがモデルタブに並びます。":
    "A dedicated integration, separate from API-key providers. It does not appear in the provider list; only the models it discovers appear in the Models tab.",
  "まだ接続されていません": "Not connected yet",
  "下のボタンからデバイスコードでログインします。ブラウザでコードを入力するだけで、パスワードをこのアプリに渡す必要はありません。":
    "Sign in with a device code using the button below. Enter the code in your browser; no password is shared with this app.",
  "ChatGPT を接続": "Connect ChatGPT",
  "先に上のスイッチで有効にしてください。":
    "Enable the integration with the switch above first.",
  "接続状況": "Connection",
  "モデルを再取得": "Refresh models",
  "モデルを表示": "View models",
  "連携は無効です。上のスイッチを有効にすると、この接続を再び使えます。":
    "The integration is disabled. Turn the switch above back on to use this connection again.",
  "無効化中": "Disabled",
  "接続情報は残っていますが、エージェントからは使用できません。有効に戻すとそのまま利用できます。":
    "The credentials are still stored, but the agent cannot use them. Re-enable the integration to resume.",
  "クレジット: 無制限": "Credits: unlimited",
  "クレジット残高:": "Credit balance:",
  "クレジットなし": "No credits",
  "使用枠に達しています。": "The usage limit has been reached.",
  "残り {percent}%": "{percent}% left",
  "使用済み {percent}%": "{percent}% used",
  "リセットを待つか、APIキー方式のProviderに切り替えてください。":
    "Wait for the reset, or switch to an API-key provider.",
  "取得:": "Updated:",
  "ログインすると 5 時間枠・週次枠の残量を表示します。":
    "Sign in to see the 5-hour and weekly usage windows.",
  "残量は OpenAI の非公開エンドポイントから取得しています。表示は保証されません。":
    "Usage comes from a private OpenAI endpoint. The figures are not guaranteed.",
  "サブスクリプション": "Subscription",
  "選択してください…": "Choose one…",
  "プリセットを選ぶと入力されます": "Filled in from the preset",
  "確認中…": "Checking…",
  "接続してモデル一覧を取得": "Connect and fetch models",

  "フォルダを選択してください": "Choose a folder first",
  "既存のフォルダを選んでください（新しいフォルダは作成されません）":
    "Pick a folder that already exists — this wizard does not create one.",
  "このプロジェクトを使う": "Use this project",
  "このフォルダを使う": "Use this folder",
  "あとから設定する": "Decide later",

  "準備ができました": "You're all set",
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
  "毎時 (0 * * * *)": "Hourly (0 * * * *)",
  "毎日 9:00": "Daily at 9:00",
  "平日 9:00 (0 9 * * mon-fri)": "Weekdays at 9:00 (0 9 * * mon-fri)",
  "毎週月曜 8:00": "Mondays at 8:00",
  "毎15分 (*/15 * * * *)": "Every 15 minutes (*/15 * * * *)",
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
  "プロジェクト外の Skills": "Skills outside the project",
  "ホームディレクトリの Skills を読み込む":
    "Load skills from the home directory",
  "~/.mix/skills、~/.claude/skills、~/.agents/skills をプロジェクトの Skills に続けて探索します。同名はプロジェクト側が優先されます。":
    "Discover ~/.mix/skills, ~/.claude/skills and ~/.agents/skills after the project's own skills. A project skill wins when names clash.",
  "{name} を {target} に書き出しました": "Exported {name} to {target}",
  "同梱リソースを読み込む": "Load bundled resources",
  "同梱リソースはありません。": "No bundled resources.",
  "バイナリファイルはプレビューできません。":
    "Binary files cannot be previewed.",

  // Browser
  "リセット": "Reset",
  "検出中…": "Scanning…",
  "ランタイム": "Runtime",
  "オンライン": "Online",
  "オフライン": "Offline",
  "不安定": "Degraded",
  "起動中…": "Starting…",
  "停止中…": "Stopping…",
  "未インストール": "Not installed",
  "不明": "Unknown",
  "ローカルランタイム": "Local runtime",
  "ローカルランタイム管理": "Local runtime management",
  "ローカルランタイムを起動": "Start local runtime",
  "起動時にローカルランタイムを検出": "Detect local runtimes on launch",
  "アプリ起動後にループバックを走査し、設定済みの Ollama / LM Studio を見つけます。":
    "Sweep loopback after launch and find configured Ollama / LM Studio.",
  "起動時にローカルランタイムを自動起動": "Auto-start local runtimes on launch",
  "設定済みのローカルプロバイダーが停止していれば、アプリから起動します。プロバイダー単位で無効化できます。":
    "Start a configured local provider when it is stopped. This can be disabled per provider.",
  "終了時にアプリ起動のランタイムを停止": "Stop app-started runtimes on exit",
  "外部で起動したランタイムは停止しません。バックグラウンドで維持したい場合はオフにしてください。":
    "Runtimes you started yourself are never stopped. Turn this off to keep app-started runtimes alive in the background.",
  "Auto でローカルモデルを優先": "Prefer local models in Auto",
  "Auto ルーティング": "Auto routing",
  "Auto がモデルを選ぶときの基準です。速度は実測 TTFT と tokens/s が貯まってから効きます。":
    "How Auto chooses a model. Speed takes effect once measured TTFT and tokens/s have accumulated.",
  "優先度": "Priority",
  "バランス（推奨）": "Balanced (recommended)",
  "速度を優先": "Prefer speed",
  "品質を優先": "Prefer quality",
  "コストを優先": "Prefer cost",
  "フォールバック": "Fallback",
  "最初の応答前に失敗した場合、別の Auto 候補で再試行します。応答が始まった後は再試行しません。":
    "Retry on another Auto candidate when a provider fails before the first response. Once output has started, it does not retry.",
  "なし": "None",
  "1回（推奨）": "Once (recommended)",
  "2回": "Twice",
  "能力・コンテキスト条件を満たすローカルモデルにスコア上の小さなボーナスを与えます。":
    "Give local models that already meet capability and context requirements a small score bonus.",
  "Ollama の keep_alive": "Ollama keep_alive",
  "Ollama に送る keep_alive（例: 5m, 30m, -1）。ロード時とローカルチャットの両方に使います。":
    "keep_alive sent to Ollama (e.g. 5m, 30m, -1). Used for explicit loads and local chat turns.",
  "検出済みのランタイムをアプリから起動・停止し、モデルの取得・ロード・削除を行えます。":
    "Start and stop detected runtimes, and pull, load or delete models from here.",
  "再検出": "Rescan",
  "アプリ起動": "App-managed",
  "起動": "Start",
  "アプリを開く": "Open app",
  "ログ": "Logs",
  "モデル名（例: qwen3:8b）": "Model name (e.g. qwen3:8b)",
  "取得": "Pull",
  "ロード": "Load",
  "アンロード": "Unload",
  "モデルがありません。上の入力欄から取得できます。":
    "No models. Pull one with the field above.",
  "直近のログ": "Recent logs",
  "ログはまだありません": "No log lines yet",
  "「{model}」を削除しますか？": 'Delete "{model}"?',
  "インストールしてください": "Install it first",
  "起動方法が不明です": "No in-app start method",
  "キャンセル中": "Cancelling",
  "キャンセル済み": "Cancelled",
  "このジョブはすでに終了しています": "This job has already finished",
  "ローカルモデルのコンテキスト上限": "Local model context ceiling",
  "ローカルモデルに渡す num_ctx の上限です。モデルの公称最大値（128K など）はそのまま使いません。":
    "Upper bound for the num_ctx sent to local models. The model's advertised maximum (often 128K) is not used as-is.",
  "自動（8K）": "Automatic (8K)",
  // Local-runtime backend errors (`ui/src/lib/localErrors.ts`).
  "「{slug}」はアプリから起動できるローカルランタイムではありません。":
    '"{slug}" is not a local runtime the app can start.',
  "{label} の起動コマンドが見つかりません。インストールと PATH を確認してください。":
    "No start command was found for {label}. Check the installation and PATH.",
  "{program} を起動できませんでした: {detail}":
    "Could not start {program}: {detail}",
  "{label} は起動しましたが応答しません: {detail}":
    "{label} started but did not respond: {detail}",
  "このランタイムはアプリの外で起動されています。ターミナルなど起動した側で停止してください。":
    "This runtime was started outside the app. Stop it from where you started it.",
  "{program} を実行できませんでした: {detail}":
    "Could not run {program}: {detail}",
  "{program} が失敗しました: {detail}": "{program} failed: {detail}",
  "このランタイムはモデルの取得に対応していません: {slug}":
    "This runtime does not support pulling models: {slug}",
  "このランタイムはモデルの削除に対応していません: {slug}":
    "This runtime does not support deleting models: {slug}",
  "このランタイムはモデルのロードに対応していません: {slug}":
    "This runtime does not support loading models: {slug}",
  "このランタイムはモデルのアンロードに対応していません: {slug}":
    "This runtime does not support unloading models: {slug}",
  "ローカルランタイムが見つかりません: {slug}":
    "Local runtime not found: {slug}",
  "モデル名が空です。": "The model name is empty.",
  "モデル名が長すぎます。": "The model name is too long.",
  "モデル名に使用できない文字が含まれています。":
    "The model name contains characters that are not allowed.",
  "アプリのパスが見つかりません: {path}": "App path not found: {path}",
  "アプリとして開けないパスです: {path}":
    "This path cannot be opened as an app: {path}",
  "ローカルランタイムへのリクエストに失敗しました: {url} ({detail})":
    "The request to the local runtime failed: {url} ({detail})",
  "ローカルランタイムがエラーを返しました: {status} {detail}":
    "The local runtime returned an error: {status} {detail}",
  "ローカルランタイムから不正な JSON を受信しました: {url} ({detail})":
    "The local runtime returned invalid JSON: {url} ({detail})",
  "モデルの取得に失敗しました: {detail}": "Model download failed: {detail}",
  "モデルの取得をキャンセルしました。": "The model download was cancelled.",
  "LM Studio の CLI (`lms`) が見つかりません。LM Studio を起動し、`~/.lmstudio/bin/lms` の存在を確認してください。":
    "The LM Studio CLI (`lms`) was not found. Start LM Studio and check that `~/.lmstudio/bin/lms` exists.",
  "`lms` を実行できませんでした: {program} ({detail})":
    "Could not run `lms`: {program} ({detail})",
  "`lms {command}` が失敗しました: {detail}":
    "`lms {command}` failed: {detail}",
  "ランタイムの状態を取得できませんでした: {endpoint} ({detail})":
    "Could not read the runtime state: {endpoint} ({detail})",
  "ランタイムのヘルスチェックに失敗しました。":
    "The runtime health check failed.",
  "ロード中": "Loaded",
  "ネイティブ": "Native",
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
  "プリセットを選んで API キーを入力してください。ChatGPT は設定 ▸ ChatGPT で接続します。":
    "Choose a preset and enter an API key. ChatGPT is connected in Settings ▸ ChatGPT.",
  "プリセット": "Preset",
  "プリセットを選択…": "Choose a preset…",
  "の設定を再利用します": " settings will be reused",
  "の設定を再利用します（キーを入力してください）":
    " settings will be reused (enter a key)",
  "直接接続": "Direct",
  "ゲートウェイ": "Gateway",
  "ローカル": "Local",
  "国内": "Japan",
  "OpenAI互換": "OpenAI compatible",
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
  "このビルドでは未対応です。データベースは平文SQLiteのため、OSのディスク暗号化（FileVault / BitLocker / LUKS）で保護してください。APIキーなどの秘密情報はマスター鍵（OSのKeychain）で暗号化してローカルに保存されます。":
    "Not available in this build. The database is plain SQLite, so protect it with OS disk encryption (FileVault / BitLocker / LUKS). Secrets such as API keys are encrypted on disk with a master seed kept in the OS keychain.",
  "SQLCipher対応は今後のバージョンで提供予定です。":
    "SQLCipher support is planned for a future release.",
  "保存時は暗号化済み": "Encrypted at rest",
  "未暗号化（平文SQLite）": "Not encrypted (plain SQLite)",
  "暗号化済み": "Encrypted",
  "未対応": "Unsupported",
  "暗号化用キーはKeychainに準備済み（未使用）":
    "Encryption key is staged in the Keychain (unused)",
  "暗号化用キーは未作成": "Encryption key has not been created",
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
  "の未コミット変更を破棄します。元に戻せません。":
    "will lose its uncommitted changes. This cannot be undone.",
  "元に戻す": "Discard",
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

  // Run lifecycle and approvals
  "停止しました": "Stopped",
  "再実行": "Run again",
  "回答を再生成": "Regenerate answer",
  "この回答を破棄して再生成": "Discard this answer and regenerate",
  "このメッセージから分岐": "Fork from this message",
  "再送信": "Send again",
  "期限切れ（拒否されます）": "Expired (will be denied)",
  "残り {time}": "{time} left",
  "この時刻を過ぎると自動的に拒否されます。":
    "The request is denied automatically after this time.",
  "承認が期限切れになったため、この操作は拒否されました。":
    "This approval expired, so the action was denied.",
  "期限が切れたため、この操作は拒否されました。再実行すると新しい承認リクエストが出ます。":
    "This approval expired and the action was denied. Run it again to get a new request.",
  "期限切れのため拒否されました。同じ操作が必要なら、スレッドで再実行してください。":
    "This approval expired and was denied. Run the action again in its thread if you still need it.",
  "先頭{shown}文字のみ表示（残り{rest}文字）":
    "Showing the first {shown} characters ({rest} remaining)",
  "先頭4,000文字のみ表示": "Showing the first 4,000 characters",
  "全文を表示": "Show all",
  "不明なスレッド": "Unknown thread",
  "Run {id}": "Run {id}",
  "スレッドを開く": "Open thread",
  "この後の{count}件のメッセージとツール実行履歴を削除して、この発言から再生成します。元に戻せません。":
    "This deletes the {count} later messages and their tool history, then regenerates from this message. This cannot be undone.",
  "削除して保存": "Delete and save",

  // Confirmation dialogs
  "このフォルダでエージェントを始める": "Start the agent in this folder",
  "使用中": "In use",
  "バックアップを復元しますか？": "Restore this backup?",
  "選択したバックアップを検証して復元の準備をします。再起動すると現在の会話・設定・記憶は復元時点の内容に置き換わります。復元後に作成したデータは元に戻せません。":
    "The selected backup will be verified and prepared for restore. After restarting, current conversations, settings and memories are replaced by the backup; anything created after it cannot be recovered.",
  "復元を適用して再起動しますか？": "Restart and apply the restore?",
  "現在のデータはバックアップの内容に置き換わります。この操作は元に戻せません。":
    "Current data will be replaced by the backup. This cannot be undone.",
  "再起動して適用": "Restart and apply",

  // Naming: agent profile vs. agent workspace / run mode
  "変更を確認": "Review changes",
  "{count}件失敗": "{count} failed",
  "設定を検索": "Search settings",
  "一致する設定がありません": "No matching settings",
  "ファイル": "Files",
  "ファイルを検索": "Search files",
  "検索結果": "Search results",
  "{name} を編集": "Edit {name}",
  "ジョブを編集": "Edit job",
  "今すぐ実行": "Run now",
  "{name} を今すぐ実行": "Run {name} now",
  "{name} を今すぐ実行しました。": "Started {name}.",
  "「常に許可」は、このプロジェクト内でこのツールの同種の操作を記憶します。":
    '"Always allow" remembers similar calls to this tool inside this project.',
  "ツールを絞った専用エージェントを定義します。エージェントモードの実行から呼び出されます。":
    "Define focused agents with a restricted tool set. Agent-mode runs invoke them.",
  "起動失敗・再試行": "Start failed — retry",
  "ランタイムがまだ検出されていません。「再検出」を実行してください。":
    "No runtime has been detected yet. Run “Detect again”.",

  // Workspace / mode naming
  "コーディング": "Coding",
  "チャットの履歴": "Chat history",
  "コーディングの履歴": "Coding history",
  "チャットの履歴を検索": "Search chat history",
  "コーディングの履歴を検索": "Search coding history",
  "チャットの履歴を検索（全履歴は ⌘K）":
    "Search chat history (⌘K searches everything)",
  "コーディングの履歴を検索（全履歴は ⌘K）":
    "Search coding history (⌘K searches everything)",
  "一致するコーディングがありません": "No matching coding threads",
  "コーディング履歴がありません": "No coding history yet",
  "新しいコーディングを始めましょう": "Start a new coding thread",
  "{label}の履歴を見る": "View {label} history",
  "ファイル編集とターミナルを使うワークスペース。":
    "A workspace for file edits and terminal work",
  "スレッド・メッセージ・コマンドを検索…":
    "Search threads, messages and commands…",
  "本文検索は3文字以上です。タイトルとコマンドを検索しています。":
    "Message search needs 3+ characters; searching titles and commands.",
  "変更は自動保存されます": "Changes save automatically",
  "設定を保存": "Save settings",
  "設定の変更が保存されていません": "Settings changes are not saved",
  "変更は保持されますが、保存するまで反映されません。終了する前に設定画面で保存してください。":
    "Your edits are kept, but they are not applied until you save. Save in Settings before quitting.",
  "移動する": "Leave",
  "設定に戻る": "Back to settings",
  "スレッド内を検索": "Find in thread",
  "前の一致": "Previous match",
  "次の一致": "Next match",
  "エージェント設定": "Agent settings",
  "自動実行": "Auto-run",
  "パネルの表示設定": "Panel display options",

  // Sidebar thread actions
  "スレッド操作": "Thread actions",
  "削除中…": "Deleting…",

  // Display preferences
  "表示設定": "Display settings",
  "表示倍率": "Zoom",
  "アプリ全体を拡大・縮小します。キーボードショートカットでも変更できます（? で一覧）。":
    "Zoom the whole app. Keyboard shortcuts also work (press ? for the list).",
  "コードを折り返す": "Wrap code",
  "長いコードブロックを横スクロールせず、画面幅で折り返して表示します。":
    "Wrap long code blocks instead of scrolling horizontally.",
  "アニメーションを減らす": "Reduce motion",
  "画面遷移や点滅を抑えます。OSの「視差効果を減らす」設定に加えて適用されます。":
    "Reduce transitions and pulsing. Applies in addition to the OS reduced-motion setting.",

  // Local data / privacy
  "ローカルデータ": "Local data",
  "入力履歴と下書きは端末の localStorage に保存されます。下書きは30日で期限切れになり、一時モードの送信は履歴に残りません。":
    "Prompt history and drafts are stored in localStorage on this device. Drafts expire after 30 days, and temporary-mode sends are not added to history.",
  "プロンプト履歴を消去 ({count})": "Clear prompt history ({count})",
  "下書きを消去 ({count})": "Clear drafts ({count})",
  "プロンプト履歴を消去しますか？": "Clear prompt history?",
  "入力欄で矢印キーを押したときに出る、過去の入力履歴を削除します。会話そのものは削除されません。":
    "Removes the past prompts shown with the arrow keys in the composer. Conversations themselves are not deleted.",
  "下書きを消去しますか？": "Clear drafts?",
  "まだ送信していない入力内容を、すべてのスレッドから削除します。元に戻せません。":
    "Removes unsent text from every thread. This cannot be undone.",
  "消去": "Clear",

  // Health
  "バックエンドに接続できません": "Cannot reach the backend",
  "ステータス": "Status",
  "診断を開く": "Open diagnostics",

  // Composer / Markdown
  "プロジェクトを選択するとファイルをメンションできます":
    "Select a project to mention files",
  "外部リンクのみ開けます": "Only external links can be opened",

  // Verification / formatting / sandbox / run steering
  "モデル（任意）": "Model (optional)",
  "空欄なら親の会話と同じモデルを使います。例: openai:gpt-5-mini":
    "Empty uses the same model as the parent conversation. Example: openai:gpt-5-mini",
  "検証と整形": "Verification and formatting",
  "検証コマンド": "Verification command",
  "編集後に実行する確認コマンドです。空欄の場合、検証ゲートは無効になります。":
    "Command run after edits to verify the change. Empty disables the verification gate.",
  "候補:": "Suggestions:",
  "整形コマンド": "Formatting command",
  "編集が入ったステップの後に自動実行する整形・lintコマンドです。":
    "Formatter/lint command run automatically after a step that changed files.",
  "検証と整形の既定値": "Verification and formatting defaults",
  "プロジェクト固有の指定がないときに使う確認コマンドです。空欄なら検証ゲートは無効です。":
    "Verification command used when a project has no override. Empty disables the gate.",
  "編集後に自動実行する整形・lintコマンドです。":
    "Formatter/lint command run automatically after edits.",
  "ターミナルのサンドボックス": "Terminal sandbox",
  "シェルをサンドボックス内で実行": "Run the shell in a sandbox",
  "シェルからのネットワーク通信も拒否":
    "Also deny network access from the shell",
  "実行中に追加指示を送る": "Send guidance while the run is working",
  "このセッションで許可": "Allow for this session",
  "実行の取り消し": "Revert this run",
  "このRunが edit / files で変更したファイルを、実行前の内容に戻します。ターミナル経由の変更は対象外です。":
    "Restores files this run changed through edit / files. Changes made through the terminal are not covered.",
  "このRunの変更を取り消しますか？": "Revert this run's changes?",
  "実行前の内容に上書きします。取り消し後に手で加えた変更も失われます。":
    "Files are overwritten with their pre-run content. Manual changes made afterwards are lost too.",
  "取り消す": "Revert",
  "実行前の状態に戻す": "Restore pre-run state",
  "{count} 件のファイルを実行前の内容に戻しました。":
    "Restored {count} file(s) to their pre-run content.",
  "履歴を圧縮しました": "Context compacted",
  "{count} 回": "{count} times",
  "直近 {kept} 件を保持": "Keeping the latest {kept}",
  "古いメッセージ {source} 件を要約し、直近 {kept} 件をそのまま保持しています。":
    "Summarised {source} older messages and kept the latest {kept} verbatim.",
  "ツール結果 {count} 件を要約に退避しました。":
    "Moved {count} tool result(s) into the summary.",
  "要約 {chars} 文字 / アーカイブ {rows} 件":
    "Summary {chars} chars / {rows} archived rows",
  "{count} 件 → {kept} 件": "{count} → {kept}",
  "以前のスレッドを読み込む": "Load older threads",
  "バックグラウンド実行": "Background execution",
  "ログイン時に起動": "Launch at login",
  "OS のログイン時に MIX-agent をトレイ常駐で起動します。スケジュールをアプリ未起動のまま実行するために必要です。":
    "Starts MIX-agent in the tray when you log in. Required for schedules to run while the app is closed.",
  "ウィンドウを閉じてもトレイに残り、スケジュールと実行中のタスクは継続します。完全に終了するにはトレイメニューの「Quit」を使います。":
    "Closing the window keeps the app in the tray so schedules and running tasks continue. Use Quit in the tray menu to exit completely.",
  "アプリを終了している間はスケジュールが実行されません。":
    "Schedules do not run while the app is closed.",
  "{count} 件の実行が中断されています": "{count} run(s) were interrupted",
  "無題のスレッド": "Untitled thread",
  "ステップ {step}": "Step {step}",
  "再開中…": "Resuming…",
  "ほか {count} 件": "{count} more",
  "チェックポイントから再開": "Resume from checkpoint",
  "再開できるチェックポイントがありません":
    "No checkpoint is available to resume from",
  "プロジェクトを選ばない場合、アプリ専用の一時領域で実行されます。":
    "Without a project, the run uses an app-private scratch workspace.",
  "実行を停止": "Stop the run",
  "{count} 件実行中": "{count} running",
  "チャットに戻る": "Back to chat",
  "コスト管理": "Cost control",
  "月次予算 (USD)": "Monthly budget (USD)",
  "無制限": "Unlimited",
  "今月の推定使用額: ${amount}": "Estimated spend this month: ${amount}",
  "この額に達するとクラウドモデルの新しい実行を開始しません（0 または空欄で無効）。ローカルモデルは対象外です。":
    "Once reached, new cloud-model runs are not started (0 or empty disables the cap). Local models are exempt.",
  "フォルダを開く": "Open folder",
  "ターミナルで開く": "Open in Terminal",
  "診断バンドルを保存": "Save diagnostics bundle",
  "サポート用バンドル": "Support bundle",
  "再起動して更新": "Restart and update",
  "再起動しています…": "Restarting…",
  "実行中のタスクがあります": "A task is still running",
  "{count} 件の実行が進行中です。終了すると実行は中断され、次回起動時にチェックポイントから再開できます。":
    "{count} run(s) are still in progress. Quitting interrupts them; they can be resumed from a checkpoint on the next launch.",
  "実行を停止して終了": "Stop the runs and quit",
  "終了しています…": "Quitting…",
  "承認待ちの保持時間（分）": "Approval prompt lifetime (minutes)",
  "エージェントが承認を待つ時間です。この時間を過ぎると自動的に拒否されます。離席することが多い場合は長めにしてください（1〜10080分）。":
    "How long the agent waits for an approval before the request is denied automatically. Increase it if you often step away (1–10080 minutes).",
  "ログイン時に起動を有効にする": "Enable launch at login",
  "bubblewrap (bwrap) を使い、ワークスペースとビルドキャッシュ以外への書き込みを拒否します。次に起動するシェルから適用されます。":
    "Uses bubblewrap (bwrap) to deny writes outside the workspace and build caches. Applies to shells started afterwards.",
  "Job Object でシェルとその子プロセスを束ね、プロセス数の上限とアプリ終了時の一括停止を強制します。ファイルシステムの書き込み隔離はOSレベルでは未対応です。":
    "Groups the shell and its children in a Job Object, enforcing a process limit and killing the tree when the app exits. Filesystem write isolation is not enforced at the OS level.",
  "macOS では sandbox-exec を使い、ワークスペースと一時ディレクトリ以外への書き込みを拒否します。次に起動するシェルから適用されます。":
    "On macOS, sandbox-exec denies writes outside the workspace and temp directories. Applies to shells started afterwards.",
  "この環境ではサンドボックスを利用できません。":
    "The sandbox is unavailable on this machine.",
  "パッケージ導入やテストが通らなくなるため既定はOFFです。信頼できないコードを動かすときだけ有効にしてください。":
    "Off by default because package installs and tests need the network. Enable it only when running untrusted code.",
  "このRunの変更をGitのスナップショットまで戻します。edit / files に加え、ターミナルが作成・変更したファイルも対象です。Run終了後に手で加えた変更も失われます。":
    "Restores this Run to its Git snapshot, covering files the terminal created or changed as well as edit / files. Manual changes made after the Run finished are lost too.",
  "{count} 件の変更を確認": "Review {count} changed file(s)",
  "Spotlight": "Spotlight",
  "macOS Spotlight": "macOS Spotlight",
  "Spotlightに索引を提供": "Publish to Spotlight",
  "会話・プロジェクト・ナレッジのタイトルをmacOSのSpotlight検索結果に表示します。":
    "Show conversation, project, and knowledge titles in macOS Spotlight results.",
  "本文も索引に含める": "Index body text too",
  "会話とナレッジの本文も検索対象にします。ツール出力・添付ファイル・認証情報は含まれません。":
    "Include conversation and knowledge body text in search. Tool output, attachments, and credentials are excluded.",
  "索引はこのMacのローカルにのみ保存されます。アプリを削除しても索引が残る場合があります。":
    "The index is stored locally on this Mac. It may remain after the app is uninstalled.",
  "この環境ではSpotlight索引を利用できません":
    "Spotlight indexing is unavailable in this environment",
  "索引の操作に失敗しました。しばらくしてから再試行してください。":
    "The index operation failed. Try again in a moment.",
  "索引の再構築はバックグラウンドで実行されます。":
    "Rebuilding runs in the background.",
  "今すぐ再構築": "Rebuild now",
  "索引から削除": "Remove from index",
  "Spotlightの索引を削除しますか？": "Remove the Spotlight index?",
  "MIX-agentが登録した索引をこのMacのSpotlightから削除します。設定を有効にしたままにすると、次回の変更時に再登録されます。":
    "Removes MIX-agent's entries from Spotlight on this Mac. If the setting stays enabled, they are added again on the next change.",
  "索引の状態": "Index status",
  "索引済み": "Indexed",
  "未処理": "Pending",
  "最終同期": "Last sync",
  "未実行": "Never",
  "本文索引": "Body indexing",
  "このブラウザの閲覧データを削除しますか？":
    "Clear this browser's browsing data?",
  "内蔵ブラウザ": "Built-in browser",
  "ページはアプリ内のネイティブ WebView で表示されます。エージェントと同じ画面を共有し、スクロール・入力・IME はそのまま動きます。":
    "The page is rendered in a native WebView inside the app. You and the agent share the same page, with native scrolling, typing and IME.",
  "Cookie とログインはこのアプリ専用のブラウザデータに保存されます。通常の Chrome とは共有されません。":
    "Cookies and sign-ins are stored in this app's own browser data and are not shared with your regular Chrome.",
  "閲覧データを削除": "Clear browsing data",
  "スレッドを開始すると使えます。": "Start a thread to use it.",
  "ネイティブブラウザを読み込んでいます…": "Loading the native browser…",
};
