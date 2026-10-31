/**
 * English text for the Docker API's stable error codes.
 *
 * The server sends `{ detail, code }`; `detail` is the canonical Japanese
 * message (also used by the Japanese UI and logs), `code` selects this table.
 * `scripts/check-error-i18n.py` fails when the server emits a code that is
 * missing here, so a new error cannot silently fall back to Japanese.
 */
export const ERROR_CODES_EN: Record<string, string> = {
  "auto.no_models": "Choose at least one model for Auto mode.",
  "feedback.auto_only": "Only answers selected by Auto can be rated.",
  "browser.install_unavailable":
    "Cannot reach the browser installer. Start the Docker deployment.",
  "browser.not_pending": "This browser action is not waiting for approval.",
  "browser.update_unavailable":
    "Cannot reach the browser updater. Start the Docker deployment.",
  "auth.csrf_invalid":
    "Your session token is invalid. Reload the page and try again.",
  "request.idempotency_key_required": "A request idempotency key is required.",
  "network.private_opt_in_required":
    "Enable “allow private connections” before using a LAN address.",
  "mcp.registry_install_unavailable":
    "Could not fetch installation details from the MCP Registry.",
  "mcp.registry_detail_unavailable": "Could not fetch MCP Registry details.",
  "mcp.registry_unavailable": "Could not reach the MCP Registry.",
  "mcp.install_failed": "The MCP server could not be installed safely.",
  "mcp.oauth_remote_only": "OAuth is available only for remote MCP servers.",
  "provider.transport_required": "Choose a provider connection type.",
  "push.subscription_invalid": "The push subscription is invalid.",
  "push.key_invalid": "The push subscription key is invalid.",
  "mcp.not_registry": "This MCP server is not from the Registry.",
  "mcp.registry_already_installed":
    "This Registry MCP server is already installed.",
  "search.url_too_long": "The SearXNG URL is too long.",
  "search.url_invalid": "Enter a public HTTPS endpoint for SearXNG.",
  "tool.definition_changed": "The tool definition has changed.",
  "model.vision_required": "Choose a model that supports vision.",
  "mcp.already_installed": "This MCP server is already installed.",
  "provider.unsupported_option":
    "This provider does not support some of the selected options.",
  "provider.base_url_required": "This provider requires a Base URL.",
  "conversation.running": "This conversation is currently running.",
  "run.already_started": "This run has already started.",
  "browser.manual_unavailable":
    "Manual control cannot be started for this run.",
  "approval.invalid": "This approval is no longer valid.",
  "conversation.trash_required":
    "Move the conversation to trash before deleting it permanently.",
  "auth.username_taken": "That username is already taken.",
  "skill.archive_too_large": "The archive must be 5 MB or smaller.",
  "conversation.restore_first":
    "Restore the conversation from the archive or trash first.",
  "provider.custom_transport_invalid":
    "Choose OpenAI-compatible, Anthropic, or Gemini for a custom provider.",
  "backup.failed":
    "Backup failed. Stop runs and background processes, then check the runner connection and size limit.",
  "backup.in_progress": "A backup or restore is in progress.",
  "egress.policy_write_failed":
    "Settings were saved, but the network policy could not be written. Retry the update.",
  "message.empty": "Enter a message.",
  "provider.model_sync_failed":
    "Could not fetch the model list. Your existing settings were kept.",
  "message.user_only_editable": "Only user messages can be edited.",
  "auth.invalid_credentials": "Incorrect username or password.",
  "auth.login_required": "Please sign in.",
  "mcp.local_only": "This action is available only for local MCP servers.",
  "provider.unknown_preset": "Unknown provider preset.",
  "run.resume_interrupted_only": "Only interrupted runs can be resumed.",
  "run.not_awaiting_budget": "This run is not waiting for a budget extension.",
  "mcp.retained_data_unavailable":
    "This MCP server cannot be reinstalled from retained data.",
  "provider.delete_models_first": "Delete the related models first.",
  "validation.invalid_input": "Check the values you entered.",
  "run.resume_agent_only":
    "Runs can be resumed only in long-running agent mode.",
  "setup.already_complete": "Initial setup is already complete.",
  "run.another_active": "Another run is already in progress.",
  "push.other_user": "This subscription belongs to another user.",
  "request.idempotency_conflict":
    "The same idempotency key cannot be reused with different input.",
  "backup.volume_mismatch":
    "The volume name for permanent deletion does not match.",
  "run.stop_before_delete": "Stop the run before deleting it.",
  "message.edit_while_running":
    "Messages cannot be edited while the run is active.",
  "mcp.distribution_missing":
    "No matching Remote, OCI, npm, or PyPI distribution was found.",
  "upload.unsupported_type":
    "Supported formats: images, PDF, text, and Markdown.",
  "memory.no_revision": "There is no Memory revision to restore.",
  "backup.restore_failed":
    "Restore failed. Check the passphrase, format, and runner connection. The backup was kept.",
  "browser.manual_inactive": "Manual control is not active.",
  "browser.manual_stop_first": "Exit manual control before approving.",
  "approval.already_decided": "This approval has already been decided.",
  "run.checkpoint_not_found": "The selected checkpoint was not found.",
  "provider.test_failed": "The connection test failed.",
  "common.invalid_action": "This action is not valid.",
  "browser.no_frame": "There is no browser view to control.",
  "provider.invalid_base_url": "Enter a valid HTTP(S) Base URL.",
  "skill.not_registered": "There are unregistered skills.",
  "tool.not_registered": "There are unregistered tools.",
  "upload.too_large": "Attachments must be 20 MB or smaller.",
  "auth.current_password_invalid": "The current password is incorrect.",
  "browser.frame_not_found": "The browser view was not found.",
  "setup.admin_exists": "An administrator already exists.",
  "run.unknown_outcome":
    "An operation has an unknown outcome. Check the external state.",
  "common.not_found": "Not found.",
  "common.backup_in_progress":
    "A backup or restore is in progress. Try again when it finishes.",
  "common.origin_not_allowed": "This request came from an unapproved origin.",
  "common.cross_site_blocked": "Cross-site requests are blocked.",
  "auth.too_many_attempts": "Too many attempts. Wait a moment and try again.",
  "agent.research_tools_required":
    "Research mode needs web search or the browser tool.",
  "agent.research_requires_agent": "Research mode must run in Agent mode.",
  "knowledge.text_too_long":
    "Reference text must be 50,000 characters or fewer in total.",
  "network.exact_domain_required":
    "Enter an exact domain name for the allowed destination.",
  // Fallbacks for dynamic details; the server adds `common.http_<status>`.
  "common.http_400": "The request was invalid.",
  "common.http_401": "Please sign in.",
  "common.http_403": "You do not have permission to do that.",
  "common.http_404": "Not found.",
  "common.http_409": "The request conflicts with the current state.",
  "common.http_413": "The file is too large.",
  "common.http_422": "Check the values you entered.",
  "common.http_429": "Too many requests. Wait a moment and try again.",
  "common.http_500": "The server encountered an error.",
  "common.http_502": "The server could not reach an upstream service.",
  "common.http_503": "The service is temporarily unavailable.",
};

export function errorCodeText(code: string | undefined): string | null {
  if (!code) return null;
  return ERROR_CODES_EN[code] ?? null;
}
