import base64
import json

import pytest

from mix_agent.providers import chatgpt
from mix_agent.providers import chatgpt_config
from mix_agent.providers.adapters import Adapter, ProviderConfigurationError


def _jwt(payload: dict) -> str:
    def encode(value: dict) -> str:
        return base64.urlsafe_b64encode(json.dumps(value).encode()).decode().rstrip("=")

    return f"{encode({'alg': 'none'})}.{encode(payload)}.sig"


def test_parse_id_token_extracts_chatgpt_claims():
    token = _jwt(
        {
            "exp": 1767225600,
            "email": "user@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct_123",
                "chatgpt_plan_type": "plus",
            },
        }
    )
    claims = chatgpt.parse_id_token(token)
    assert claims == {
        "account_id": "acct_123",
        "plan_type": "plus",
        "email": "user@example.com",
        "expires_at": 1767225600.0,
    }


def test_parse_id_token_tolerates_garbage():
    assert chatgpt.parse_id_token("not-a-jwt") == {
        "account_id": "",
        "plan_type": "",
        "email": "",
        "expires_at": 0.0,
    }


def test_credentials_from_legacy_api_key_do_not_leak():
    creds = chatgpt.ChatGptCredentials.from_json("sk-legacy-key")
    assert creds.access_token == ""
    assert creds.refresh_token == ""
    assert creds.account_id == ""


def test_credentials_need_refresh_near_expiry():
    creds = chatgpt.ChatGptCredentials(access_token="a", refresh_token="r", expires_at=1000)
    assert creds.needs_refresh(now=1000 - chatgpt.REFRESH_SKEW_SECONDS + 1) is True
    assert creds.needs_refresh(now=1000 - chatgpt.REFRESH_SKEW_SECONDS - 1) is False
    assert chatgpt.ChatGptCredentials().needs_refresh(now=1) is True


def test_normalize_usage_maps_primary_and_secondary_windows():
    quota = chatgpt.normalize_usage(
        {
            "plan_type": "plus",
            "rate_limit": {
                "allowed": True,
                "limit_reached": False,
                "primary_window": {
                    "used_percent": 42,
                    "limit_window_seconds": 18000,
                    "reset_at": 1767225600,
                },
                "secondary_window": {
                    "used_percent": 120,
                    "limit_window_seconds": 604800,
                    "reset_at": 1767800000,
                },
            },
        }
    )
    assert quota["plan_type"] == "plus"
    assert quota["limit_reached"] is False
    assert [window["id"] for window in quota["windows"]] == ["5h", "week"]
    assert quota["windows"][0]["used_percent"] == 42.0
    # A backend that over-reports is clamped instead of drawing an invalid bar.
    assert quota["windows"][1]["used_percent"] == 100.0


def test_normalize_usage_accepts_window_minutes_events():
    quota = chatgpt.normalize_usage(
        {
            "rate_limit": {
                "primary_window": {
                    "used_percent": 10,
                    "window_minutes": 300,
                    "reset_at": 1767225600,
                }
            }
        }
    )
    assert quota["windows"][0]["id"] == "5h"
    assert quota["windows"][0]["window_minutes"] == 300


def test_quota_from_headers_reads_credits():
    quota = chatgpt.quota_from_headers(
        {
            "x-codex-primary-used-percent": "12.5",
            "x-codex-primary-window-minutes": "300",
            "x-codex-primary-reset-at": "1767225600",
            "x-codex-credits-has-credits": "true",
            "x-codex-credits-unlimited": "false",
            "x-codex-credits-balance": "10",
        }
    )
    assert quota is not None
    assert quota["windows"][0]["used_percent"] == 12.5
    assert quota["credits"] == {
        "has_credits": True,
        "unlimited": False,
        "balance": "10",
    }


def test_quota_from_headers_ignores_unrelated_responses():
    assert chatgpt.quota_from_headers({"content-type": "text/event-stream"}) is None


class _Response:
    def __init__(self, status_code, payload, text=""):
        self.status_code = status_code
        self._payload = payload
        self.text = text

    @property
    def is_success(self):
        return 200 <= self.status_code < 300

    def json(self):
        return self._payload


class _Client:
    def __init__(self, response):
        self.response = response

    async def __aenter__(self):
        return self

    async def __aexit__(self, *args):
        return False

    async def post(self, *args, **kwargs):
        return self.response


@pytest.mark.asyncio
async def test_start_device_login_returns_a_one_time_code(monkeypatch):
    monkeypatch.setattr(
        chatgpt.httpx,
        "AsyncClient",
        lambda **kwargs: _Client(
            _Response(
                200,
                {
                    "device_auth_id": "device-1",
                    "user_code": "ABCD-EFGH",
                    "interval": "5",
                },
            )
        ),
    )
    started = await chatgpt.start_device_login("provider-1", "owner-1")
    assert started["verification_url"] == chatgpt.DEVICE_VERIFICATION_URL
    assert started["user_code"] == "ABCD-EFGH"
    assert started["interval"] == 5
    assert chatgpt.cancel_device_login(started["login_id"], "owner-1") is True
    assert chatgpt.cancel_device_login(started["login_id"], "owner-2") is False


@pytest.mark.asyncio
async def test_device_start_classifies_a_disabled_server(monkeypatch):
    monkeypatch.setattr(
        chatgpt.httpx,
        "AsyncClient",
        lambda **kwargs: _Client(_Response(404, {})),
    )
    with pytest.raises(chatgpt.ChatGptError) as error:
        await chatgpt.start_device_login("provider-1", "owner-1")
    assert error.value.code == "chatgpt_device_unavailable"


def test_chatgpt_transport_adds_reasoning_and_prompt_cache():
    adapter = Adapter(
        {
            "id": "provider-1",
            "kind": "openai",
            "preset_id": "chatgpt",
            "base_url": chatgpt_config.CHATGPT_BASE_URL,
        },
        "",
    )
    args = adapter._responses_args(
        "gpt-5-codex",
        [{"role": "user", "content": "hi"}],
        [],
        "chat",
        {"_session_id": "run-1"},
    )
    assert args["prompt_cache_key"] == "run-1"
    assert args["include"] == ["reasoning.encrypted_content"]
    assert args["reasoning"]["effort"] == "medium"
    # An empty tool list is omitted rather than sent as [].
    assert "tools" not in args


def test_chatgpt_transport_refuses_a_foreign_base_url():
    with pytest.raises(ProviderConfigurationError, match="invalid_chatgpt_base_url"):
        Adapter(
            {
                "id": "provider-1",
                "kind": "openai",
                "preset_id": "chatgpt",
                "base_url": "https://evil.example/collect",
            },
            "",
        )


def test_normalize_usage_treats_allowed_false_as_limit_reached():
    quota = chatgpt.normalize_usage(
        {
            "rate_limit": {
                "allowed": False,
                "limit_reached": False,
                "primary_window": {
                    "used_percent": 100,
                    "limit_window_seconds": 18000,
                },
            }
        }
    )
    assert quota["limit_reached"] is True


def test_credentials_round_trip_the_auth_failed_flag():
    raw = json.dumps(
        {
            "access_token": "a",
            "refresh_token": "r",
            "auth_failed": True,
        }
    )
    creds = chatgpt.ChatGptCredentials.from_json(raw)
    assert creds.auth_failed is True
    assert json.loads(creds.to_json())["auth_failed"] is True


@pytest.mark.asyncio
async def test_quota_falls_back_to_the_last_snapshot_on_a_transient_error(monkeypatch):
    auth = chatgpt.ChatGptAuth(
        "",
        json.dumps(
            {
                "access_token": "a",
                "refresh_token": "r",
                "expires_at": chatgpt.time.time() + 3600,
            }
        ),
        "provider-1",
    )
    cached = {
        "plan_type": "plus",
        "captured_at": "2026-01-01T00:00:00+00:00",
        "windows": [],
        "credits": None,
        "limit_reached": False,
    }
    chatgpt._write_quota_cache("provider-1", cached)

    async def fail(_creds):
        raise chatgpt.ChatGptError("chatgpt_quota_failed", "timeout")

    monkeypatch.setattr(chatgpt, "fetch_usage", fail)
    assert await auth.quota(force=True) == cached

    chatgpt._drop_quota_cache("provider-1")
    with pytest.raises(chatgpt.ChatGptError):
        await auth.quota(force=True)


@pytest.mark.asyncio
async def test_a_permanent_refresh_failure_is_persisted(monkeypatch):
    auth = chatgpt.ChatGptAuth(
        "",
        json.dumps({"access_token": "a", "refresh_token": "r"}),
    )
    saved: list[str] = []

    async def expire(_creds):
        raise chatgpt.ChatGptError(chatgpt.AUTH_EXPIRED, "refresh token is gone")

    async def persist():
        saved.append(auth.creds.to_json())
        return True

    monkeypatch.setattr(chatgpt, "refresh_credentials", expire)
    monkeypatch.setattr(auth, "_persist", persist)
    with pytest.raises(chatgpt.ChatGptError):
        await auth.refresh()
    assert auth.creds.auth_failed is True
    assert saved and json.loads(saved[0])["auth_failed"] is True


@pytest.mark.asyncio
async def test_refresh_adopts_a_persisted_token_even_near_expiry(monkeypatch):
    """A rotated refresh token must win even when it is inside the refresh skew.

    The previous implementation only adopted a persisted token when it was not
    near expiry, then refreshed with the stale (already rotated) refresh token.
    OpenAI rejects that with ``invalid_grant`` and logs the user out.
    """
    auth = chatgpt.ChatGptAuth(
        "",
        json.dumps(
            {
                "access_token": "old-access",
                "refresh_token": "old-refresh",
                "expires_at": chatgpt.time.time() + 3600,
            }
        ),
        "provider-1",
    )
    persisted = chatgpt.ChatGptCredentials(
        access_token="new-access",
        refresh_token="new-refresh",
        # Inside REFRESH_SKEW_SECONDS, so it still needs a refresh.
        expires_at=chatgpt.time.time() + 60,
    )
    seen: list[str] = []

    def load():
        return persisted

    async def refresh(creds):
        seen.append(creds.refresh_token)
        return chatgpt.ChatGptCredentials(
            access_token="final-access",
            refresh_token="final-refresh",
            expires_at=chatgpt.time.time() + 3600,
        )

    async def persist():
        return True

    monkeypatch.setattr(auth, "_load_persisted", load)
    monkeypatch.setattr(chatgpt, "refresh_credentials", refresh)
    monkeypatch.setattr(auth, "_persist", persist)
    await auth.refresh()
    assert seen == ["new-refresh"]
    assert auth.creds.access_token == "final-access"


@pytest.mark.asyncio
async def test_a_failed_login_is_not_refreshed_on_every_turn(monkeypatch):
    auth = chatgpt.ChatGptAuth(
        "",
        json.dumps(
            {
                "access_token": "a",
                "refresh_token": "r",
                "expires_at": 0,
                "auth_failed": True,
            }
        ),
    )

    async def unexpected(_creds):
        raise AssertionError("a permanently failed login must not retry refresh")

    monkeypatch.setattr(chatgpt, "refresh_credentials", unexpected)
    with pytest.raises(chatgpt.ChatGptError):
        await auth.token()


def test_normalize_usage_accepts_loose_boolean_flags():
    for limit_reached, allowed in ((1, None), ("true", None), (None, "false"), (None, 0)):
        quota = chatgpt.normalize_usage(
            {
                "rate_limit": {
                    "limit_reached": limit_reached,
                    "allowed": allowed,
                    "primary_window": {"used_percent": 100, "limit_window_seconds": 18000},
                }
            }
        )
        assert quota["limit_reached"] is True, (limit_reached, allowed)


def test_six_hour_window_is_not_labelled_five_hours():
    quota = chatgpt.normalize_usage(
        {"rate_limit": {"primary_window": {"used_percent": 1, "limit_window_seconds": 6 * 3600}}}
    )
    assert quota["windows"][0]["id"] == "6h"


@pytest.mark.asyncio
async def test_poll_device_login_reports_an_expired_code_immediately(monkeypatch):
    chatgpt._DEVICE_LOGINS["expired-login"] = {
        "provider_id": "provider-1",
        "owner_id": "owner-1",
        "device_auth_id": "device-1",
        "user_code": "CODE",
        "interval": 5,
        "created_at": chatgpt.time.time(),
        "expires_at": chatgpt.time.time() + 60,
        "next_poll_at": 0,
        "poll_failures": 0,
        "status": "pending",
    }
    monkeypatch.setattr(
        chatgpt.httpx,
        "AsyncClient",
        lambda **kwargs: _Client(_Response(403, {}, text="device_code_expired: the code has expired")),
    )
    result = await chatgpt.poll_device_login("expired-login", "owner-1")
    assert result["status"] == "error"
    assert result["error"] == "chatgpt_login_expired"
    chatgpt.cancel_device_login("expired-login", "owner-1")


@pytest.mark.asyncio
async def test_poll_device_login_keeps_the_code_alive_on_a_server_error(monkeypatch):
    chatgpt._DEVICE_LOGINS["retry-login"] = {
        "provider_id": "provider-1",
        "owner_id": "owner-1",
        "device_auth_id": "device-1",
        "user_code": "CODE",
        "interval": 5,
        "created_at": chatgpt.time.time(),
        "expires_at": chatgpt.time.time() + 60,
        "next_poll_at": 0,
        "poll_failures": 0,
        "status": "pending",
    }
    monkeypatch.setattr(
        chatgpt.httpx,
        "AsyncClient",
        lambda **kwargs: _Client(_Response(503, {})),
    )
    result = await chatgpt.poll_device_login("retry-login", "owner-1")
    assert result["status"] == "pending"
    assert chatgpt._DEVICE_LOGINS["retry-login"]["poll_failures"] == 1
    chatgpt.cancel_device_login("retry-login", "owner-1")


def test_normalize_usage_coerces_credits_to_the_api_shape():
    quota = chatgpt.normalize_usage(
        {
            "rate_limit": {"primary_window": {"used_percent": 1, "limit_window_seconds": 18000}},
            "credits": {"has_credits": 1, "unlimited": "false", "balance": 12.5},
        }
    )
    assert quota["credits"] == {
        "has_credits": True,
        "unlimited": False,
        "balance": "12.5",
    }
    null_balance = chatgpt.normalize_usage(
        {
            "rate_limit": {},
            "credits": {"has_credits": True, "balance": None},
        }
    )
    assert null_balance["credits"]["balance"] == ""


def test_quota_from_headers_coerces_credits_and_keeps_unknown_reset_hidden():
    quota = chatgpt.quota_from_headers(
        {
            "x-codex-primary-used-percent": "5",
            "x-codex-primary-window-minutes": "0",
            "x-codex-credits-has-credits": "True",
            "x-codex-credits-unlimited": "1",
            "x-codex-credits-balance": "10",
        }
    )
    assert quota["windows"][0]["id"] == "5h"
    assert quota["windows"][0]["resets_at"] == 0
    assert quota["credits"] == {
        "has_credits": True,
        "unlimited": True,
        "balance": "10",
    }


def test_chatgpt_preset_never_reports_an_embedding_model():
    adapter = Adapter(
        {
            "kind": "openai",
            "preset_id": "chatgpt",
            "base_url": chatgpt_config.CHATGPT_BASE_URL,
            "extra_config": {},
            "embedding_model": "text-embedding-3-small",
        },
        "oauth-secret",
    )
    assert adapter.embedding_model() is None


@pytest.mark.asyncio
async def test_poll_device_login_backs_off_on_a_rate_limit(monkeypatch):
    chatgpt._DEVICE_LOGINS["limited-login"] = {
        "provider_id": "provider-1",
        "owner_id": "owner-1",
        "device_auth_id": "device-1",
        "user_code": "CODE",
        "interval": 5,
        "created_at": chatgpt.time.time(),
        "expires_at": chatgpt.time.time() + 60,
        "next_poll_at": 0,
        "poll_failures": 0,
        "status": "pending",
    }
    monkeypatch.setattr(
        chatgpt.httpx,
        "AsyncClient",
        lambda **kwargs: _Client(_Response(429, {})),
    )
    result = await chatgpt.poll_device_login("limited-login", "owner-1")
    assert result["status"] == "pending"
    state = chatgpt._DEVICE_LOGINS["limited-login"]
    assert state["poll_failures"] == 1
    assert state["next_poll_at"] > chatgpt.time.time()
    chatgpt.cancel_device_login("limited-login", "owner-1")


def test_embedding_provider_selection_skips_chatgpt():
    from mix_agent.memory import embeddings as mem_embeddings

    class Row:
        def __init__(self, data):
            self.data = data

    chatgpt_row = Row(
        {
            "kind": "openai",
            "preset_id": "chatgpt",
            "base_url": chatgpt_config.CHATGPT_BASE_URL,
            "extra_config": {},
        }
    )
    openai_row = Row(
        {
            "kind": "openai",
            "preset_id": "openai",
            "base_url": "https://api.openai.com/v1",
            "extra_config": {},
        }
    )
    assert not mem_embeddings.provider_can_embed(chatgpt_row)
    assert mem_embeddings.provider_can_embed(openai_row)
