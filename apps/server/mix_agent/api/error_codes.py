"""Stable error codes for the Docker edition's HTTP API.

The server still returns the human-readable Japanese message in ``detail`` so
existing clients and logs keep working, but every known message also carries a
stable ``code``.  The web client translates the code instead of matching on the
sentence, so editing a message no longer silently drops its English
translation.

``scripts/check-error-i18n.py`` keeps this table, the messages raised in the
code, and ``apps/web/src/app/errorCodes.ts`` in sync.
"""

from fastapi import HTTPException

# Canonical Japanese message -> stable machine-readable code.
ERROR_CODES: dict[str, str] = {
    "Autoで使用可能なモデルを設定してください": "auto.no_models",
    "Autoで選択された回答のみ評価できます": "feedback.auto_only",
    "Browser導入サービスに接続できません。Docker deploymentを起動してください。": "browser.install_unavailable",
    "Browser操作の承認待ちではありません": "browser.not_pending",
    "Browser更新サービスに接続できません。Docker deploymentを起動してください。": "browser.update_unavailable",
    "CSRFトークンが一致しません": "auth.csrf_invalid",
    "Idempotency-Keyが必要です": "request.idempotency_key_required",
    "LAN接続先は「プライベート接続を許可」を明示してください": "network.private_opt_in_required",
    "MCP Registryから導入情報を取得できません": "mcp.registry_install_unavailable",
    "MCP Registryの詳細を取得できません": "mcp.registry_detail_unavailable",
    "MCP Registryを取得できません": "mcp.registry_unavailable",
    "MCPを安全に導入できませんでした": "mcp.install_failed",
    "OAuthはRemote MCP専用です": "mcp.oauth_remote_only",
    "Providerの接続方式を指定してください": "provider.transport_required",
    "Push購読が不正です": "push.subscription_invalid",
    "Push購読キーが不正です": "push.key_invalid",
    "Registry MCPではありません": "mcp.not_registry",
    "Registry MCPは導入済みです": "mcp.registry_already_installed",
    "SearXNGのURLが長すぎます": "search.url_too_long",
    "SearXNGのURLは公開HTTPSエンドポイントを指定してください": "search.url_invalid",
    "Tool定義が変更されています": "tool.definition_changed",
    "Vision対応モデルを選択してください": "model.vision_required",
    "このMCPは導入済みです": "mcp.already_installed",
    "このProviderでは使えない追加設定があります": "provider.unsupported_option",
    "このProviderはBase URLを指定してください": "provider.base_url_required",
    "この会話は実行中です": "conversation.running",
    "この実行はすでに開始されています": "run.already_started",
    "この実行は手動操作を開始できません": "browser.manual_unavailable",
    "この承認は有効ではありません": "approval.invalid",
    "ごみ箱に移動してから完全削除してください": "conversation.trash_required",
    "そのユーザー名はすでに使われています": "auth.username_taken",
    "アーカイブは5MBまでです": "skill.archive_too_large",
    "アーカイブまたはごみ箱から復元してから続けてください": "conversation.restore_first",
    "カスタムはOpenAI互換・Anthropic・Geminiから選択してください": "provider.custom_transport_invalid",
    "クロスサイトリクエストは拒否されました": "common.cross_site_blocked",
    "バックアップできません。実行・バックグラウンドProcessを停止し、Runner接続とサイズ上限を確認してください。": "backup.failed",
    "バックアップ処理中です": "backup.in_progress",
    "バックアップ／復元処理中です": "common.backup_in_progress",
    "メッセージを入力してください": "message.empty",
    "モデル一覧を取得できません。既存設定は保持されています。": "provider.model_sync_failed",
    "ユーザーのメッセージのみ編集できます": "message.user_only_editable",
    "ユーザー名またはパスワードが違います": "auth.invalid_credentials",
    "ログインしてください": "auth.login_required",
    "ローカルMCP専用の操作です": "mcp.local_only",
    "不明なProviderプリセットです": "provider.unknown_preset",
    "中断した実行だけ再開できます": "run.resume_interrupted_only",
    "許可されていないOriginです": "common.origin_not_allowed",
    "予算延長要求中の実行ではありません": "run.not_awaiting_budget",
    "保持データから再導入できる状態ではありません": "mcp.retained_data_unavailable",
    "先に関連モデルを削除してください": "provider.delete_models_first",
    "入力項目を確認してください": "validation.invalid_input",
    "再開は長作業モードでのみ利用できます": "run.resume_agent_only",
    "初期設定は完了しています": "setup.already_complete",
    "別の実行が進行中です": "run.another_active",
    "別ユーザーの購読です": "push.other_user",
    "同じIdempotency-Keyで異なる入力は送信できません": "request.idempotency_conflict",
    "完全削除するvolume名が一致しません": "backup.volume_mismatch",
    "実行を停止してから削除してください": "run.stop_before_delete",
    "実行中はメッセージを編集できません": "message.edit_while_running",
    "対応するRemote、OCI、npm、PyPI配布がありません": "mcp.distribution_missing",
    "対応形式: 画像・PDF・テキスト・Markdown": "upload.unsupported_type",
    "復元できるMemory revisionがありません": "memory.no_revision",
    "復元に失敗しました。パスフレーズ、形式、Runner接続を確認してください。退避データは保持されます。": "backup.restore_failed",
    "手動操作は有効ではありません": "browser.manual_inactive",
    "手動操作を終了してから承認してください": "browser.manual_stop_first",
    "承認は決定済みです": "approval.already_decided",
    "指定したチェックポイントが見つかりません": "run.checkpoint_not_found",
    "接続テストに失敗しました。": "provider.test_failed",
    "操作が無効です": "common.invalid_action",
    "操作できるブラウザ画面がありません": "browser.no_frame",
    "有効なHTTP(S) Base URLを指定してください": "provider.invalid_base_url",
    "未登録Skillがあります": "skill.not_registered",
    "未登録Toolがあります": "tool.not_registered",
    "添付は20MBまでです": "upload.too_large",
    "現在のパスワードが違います": "auth.current_password_invalid",
    "画面が見つかりません": "browser.frame_not_found",
    "管理者は作成済みです": "setup.admin_exists",
    "結果不明の操作があります。外部状態を確認してください": "run.unknown_outcome",
    "見つかりません": "common.not_found",
    "試行回数が多すぎます。しばらく待ってから再試行してください。": "auth.too_many_attempts",
    "調査モードにはWeb検索またはBrowser Toolが必要です": "agent.research_tools_required",
    "調査モードはAgentモードで実行してください": "agent.research_requires_agent",
    "資料テキストは合計5万文字以内にしてください": "knowledge.text_too_long",
    "通信許可先は正確なドメイン名で指定してください": "network.exact_domain_required",
}


_MESSAGES_BY_CODE = {code: message for message, code in ERROR_CODES.items()}


def code_for_detail(detail) -> str | None:
    """Return the stable code for a known message, or ``None``.
    Dynamic messages (for example ``str(ValueError)`` from a validator) are not
    in the table; callers fall back to a status-based generic code so the UI can
    still show a localized message instead of raw Japanese.
    """
    if not isinstance(detail, str):
        return None
    return ERROR_CODES.get(detail)


def message_for(code: str) -> str | None:
    """Canonical Japanese message for a code (``None`` for unknown codes)."""
    return _MESSAGES_BY_CODE.get(code)


class APIError(HTTPException):
    """An ``HTTPException`` that carries its code explicitly.

    The message-keyed ``ERROR_CODES`` table keeps existing call sites working,
    but new code should prefer this class: renaming the Japanese sentence can
    then never change or drop the machine contract.
    """

    def __init__(self, status_code: int, code: str, detail: str | None = None, headers=None):
        super().__init__(
            status_code=status_code,
            detail=detail or message_for(code) or code,
            headers=headers,
        )
        self.code = code
