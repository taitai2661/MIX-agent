"""Constants for the unofficial ChatGPT / Codex integration.

Kept free of database and auth imports so the provider catalog and OpenAPI
tooling can import the preset without bootstrapping server configuration.
"""

AUTH_ISSUER = "https://auth.openai.com"
CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann"
SCOPES = "openid profile email offline_access"
CHATGPT_BASE_URL = "https://chatgpt.com/backend-api/codex"
CHATGPT_USAGE_URL = "https://chatgpt.com/backend-api/wham/usage"

# Honest default. Some deployments may need the Codex CLI's own originator;
# that is an explicit opt-in because it misrepresents the client.
ORIGINATOR = "mix_agent"
ORIGINATOR_ENV = "MIX_CODEX_ORIGINATOR"

DEVICE_VERIFICATION_URL = f"{AUTH_ISSUER}/codex/device"
DEVICE_REDIRECT_URI = f"{AUTH_ISSUER}/deviceauth/callback"
DEVICE_TTL_SECONDS = 15 * 60
REFRESH_SKEW_SECONDS = 5 * 60
QUOTA_TTL_SECONDS = 60
LOGIN_POLL_FLOOR_SECONDS = 2

AUTH_REQUIRED = "chatgpt_auth_required"
AUTH_EXPIRED = "chatgpt_auth_expired"
