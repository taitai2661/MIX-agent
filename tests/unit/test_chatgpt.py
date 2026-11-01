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
    def __init__(self, status_code, payload):
        self.status_code = status_code
        self._payload = payload

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
