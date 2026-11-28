/// Searchable settings fields.
///
/// The settings sidebar search used to filter tabs only: typing "予算" found
/// the 一般 tab, then left the user to scroll a long page. These anchors are a
/// curated index of the fields people actually search for. Selecting one opens
/// its tab and scrolls the matching card into view with a short highlight.
///
/// The label is the same Japanese literal used by the component (and therefore
/// an i18n key). `keywords` adds English and synonyms that are not part of the
/// visible label.

import type { SettingsTab } from "./ui";

export interface SettingsAnchor {
  /** Stable id for React keys and tests. */
  id: string;
  tab: SettingsTab;
  /** i18n key / Japanese label rendered in the target card. */
  label: string;
  keywords?: string[];
}

export const SETTINGS_ANCHORS: readonly SettingsAnchor[] = [
  // 一般
  { id: "general-setup", tab: "general", label: "初期設定" },
  {
    id: "general-verify",
    tab: "general",
    label: "検証コマンド",
    keywords: ["verify", "test", "lint"],
  },
  {
    id: "general-format",
    tab: "general",
    label: "整形コマンド",
    keywords: ["format", "prettier", "formatter"],
  },
  {
    id: "general-autostart",
    tab: "general",
    label: "ログイン時に起動",
    keywords: ["autostart", "login", "startup"],
  },
  {
    id: "general-approval-retention",
    tab: "general",
    label: "承認待ちの保持時間（分）",
    keywords: ["approval", "timeout", "承認"],
  },
  {
    id: "general-sandbox",
    tab: "general",
    label: "ターミナルのサンドボックス",
    keywords: ["sandbox", "terminal", "shell"],
  },
  {
    id: "general-budget",
    tab: "general",
    label: "月次予算 (USD)",
    keywords: ["budget", "cost", "spend", "予算"],
  },
  {
    id: "general-send-key",
    tab: "general",
    label: "メッセージ送信キー",
    keywords: ["enter", "shortcut", "send"],
  },
  {
    id: "general-auto-routing",
    tab: "general",
    label: "Auto ルーティング",
    keywords: ["routing", "auto", "fallback"],
  },
  {
    id: "general-local-runtime",
    tab: "general",
    label: "ローカルランタイム",
    keywords: ["ollama", "lm studio", "local"],
  },
  {
    id: "general-memory",
    tab: "general",
    label: "メモリ",
    keywords: ["memory", "remember"],
  },
  {
    id: "general-local-data",
    tab: "general",
    label: "ローカルデータ",
    keywords: ["drafts", "localstorage", "下書き"],
  },
  {
    id: "general-db-encryption",
    tab: "general",
    label: "データベースの暗号化",
    keywords: ["sqlcipher", "encryption", "db"],
  },

  // 外観
  { id: "appearance-theme", tab: "appearance", label: "テーマ" },
  {
    id: "appearance-accent",
    tab: "appearance",
    label: "アクセントカラー",
    keywords: ["accent", "color"],
  },
  {
    id: "appearance-zoom",
    tab: "appearance",
    label: "表示倍率",
    keywords: ["zoom", "scale", "font size"],
  },
  {
    id: "appearance-wrap",
    tab: "appearance",
    label: "コードを折り返す",
    keywords: ["wrap", "code"],
  },
  { id: "appearance-language", tab: "appearance", label: "言語" },

  // モデルと接続
  {
    id: "providers-add",
    tab: "providers",
    label: "プロバイダーを追加",
    keywords: ["provider", "api key", "base url"],
  },
  {
    id: "providers-api-key",
    tab: "providers",
    label: "API キー",
    keywords: ["api key", "key", "secret", "token"],
  },
  {
    id: "providers-chatgpt",
    tab: "providers",
    label: "ChatGPT（非公式・専用連携）",
    keywords: ["chatgpt", "openai"],
  },
  {
    id: "models-list",
    tab: "models",
    label: "モデル",
    keywords: ["model", "context", "vision"],
  },

  // エージェント
  {
    id: "tools-rules",
    tab: "tools",
    label: "スコープ付きルール",
    keywords: ["permission", "allow", "deny", "承認"],
  },
  {
    id: "mcp-add",
    tab: "mcp",
    label: "MCP サーバを追加",
    keywords: ["mcp", "server", "stdio"],
  },
  {
    id: "skills-import",
    tab: "skills",
    label: "取り込む",
    keywords: ["skill"],
  },
  {
    id: "memory-add",
    tab: "memory",
    label: "Memory を追加",
    keywords: ["memory"],
  },
  {
    id: "knowledge-import",
    tab: "knowledge",
    label: "取り込む",
    keywords: ["knowledge", "document", "rag"],
  },

  // アプリ
  { id: "browser-builtin", tab: "browser", label: "内蔵ブラウザ" },
  {
    id: "network-local",
    tab: "browser",
    label: "ブラウザのローカルアクセス",
    keywords: ["localhost", "network"],
  },
  {
    id: "updates-app",
    tab: "updates",
    label: "アプリのアップデート",
    keywords: ["update", "version"],
  },
  {
    id: "backup-restore",
    tab: "backup",
    label: "バックアップ / 復元",
    keywords: ["backup", "restore"],
  },
  {
    id: "spotlight-macos",
    tab: "spotlight",
    label: "macOS Spotlight",
    keywords: ["spotlight", "index"],
  },

  // 詳細・診断
  {
    id: "diagnostics-db",
    tab: "diagnostics",
    label: "データベースの暗号化",
    keywords: ["diagnostics", "sqlcipher"],
  },
  {
    id: "diagnostics-logs",
    tab: "diagnostics",
    label: "最近の実行ログ",
    keywords: ["logs", "run", "error"],
  },
];

/** Anchors whose label or keywords match `query` (case-insensitive). */
export function matchSettingsAnchors(
  query: string,
  translate: (key: string) => string,
  limit = 8,
): SettingsAnchor[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  return SETTINGS_ANCHORS.filter((anchor) => {
    const haystack = [
      anchor.label,
      translate(anchor.label),
      ...(anchor.keywords ?? []),
    ]
      .join(" ")
      .toLowerCase();
    return haystack.includes(q);
  }).slice(0, limit);
}
