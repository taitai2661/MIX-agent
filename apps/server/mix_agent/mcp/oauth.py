"""OAuth 2.1 client for Remote MCP with CIMD-first registration."""

from __future__ import annotations

import base64
import hashlib
import json
import secrets
import time
import re
from urllib.parse import urlencode, urlsplit

from mix_agent import config
from mix_agent.tools.network import fetch_public, request_public


def callback_url() -> str:
    return config.PUBLIC_ORIGIN + "/api/v1/mcp/oauth/callback"


def client_metadata_url() -> str:
    return config.PUBLIC_ORIGIN + "/api/v1/mcp/oauth/client-metadata.json"


def client_metadata() -> dict:
    return {
        "client_id": client_metadata_url(),
        "client_name": "MIX agent",
        "redirect_uris": [callback_url()],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": "web" if config.PUBLIC_ORIGIN.startswith("https://") else "native",
    }


async def _json(url: str) -> dict:
    value = json.loads(await fetch_public(url, max_bytes=512_000))
    if not isinstance(value, dict):
        raise ValueError("Invalid OAuth metadata")
    return value


async def discover(resource_url: str) -> dict:
    parsed = urlsplit(resource_url)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
        raise ValueError("OAuth resource must use public HTTPS")
    resource_origin = f"https://{parsed.netloc}"
    resource_metadata_url = ""
    try:
        _, headers, _ = await request_public("POST", resource_url, headers={"MCP-Protocol-Version": "2026-07-28", "Content-Type": "application/json"}, json={})
        challenge = headers.get("www-authenticate", "")
        match = re.search(r'resource_metadata="([^"\r\n]+)"', challenge)
        if match:
            resource_metadata_url = match.group(1)
    except Exception:
        pass
    resource_metadata = await _json(resource_metadata_url or resource_origin + "/.well-known/oauth-protected-resource")
    resource = str(resource_metadata.get("resource") or resource_url)
    issuers = resource_metadata.get("authorization_servers") or []
    if not issuers:
        raise ValueError("OAuth authorization server metadata is missing")
    issuer = str(issuers[0]).rstrip("/")
    issuer_url = urlsplit(issuer)
    if issuer_url.scheme != "https" or not issuer_url.hostname:
        raise ValueError("Invalid OAuth issuer")
    metadata = None
    for suffix in ("/.well-known/oauth-authorization-server", "/.well-known/openid-configuration"):
        try:
            metadata = await _json(issuer + suffix)
            break
        except Exception:
            continue
    if not metadata or str(metadata.get("issuer", "")).rstrip("/") != issuer:
        raise ValueError("OAuth issuer metadata mismatch")
    return {"resource": resource, "issuer": issuer, "metadata": metadata}


async def register(discovery: dict, manual: dict | None = None) -> dict:
    metadata = discovery["metadata"]
    if manual and manual.get("client_id"):
        return {"method": "pre_registered", "client_id": manual["client_id"], "client_secret": manual.get("client_secret", "")}
    if metadata.get("client_id_metadata_document_supported") and config.PUBLIC_ORIGIN.startswith("https://"):
        return {"method": "cimd", "client_id": client_metadata_url(), "client_secret": ""}
    endpoint = metadata.get("registration_endpoint")
    if not endpoint:
        raise ValueError("CIMD is unavailable; enter a pre-registered OAuth client")
    status, _, body = await request_public("POST", endpoint, json=client_metadata())
    if status < 200 or status >= 300:
        raise ValueError("Legacy dynamic client registration failed")
    registered = json.loads(body)
    return {"method": "legacy_dcr", "client_id": registered["client_id"], "client_secret": registered.get("client_secret", "")}


def authorization(discovery: dict, registration: dict, state: str, verifier: str, scopes: list[str]) -> str:
    endpoint = discovery["metadata"].get("authorization_endpoint")
    if not endpoint:
        raise ValueError("OAuth authorization endpoint is missing")
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    query = {
        "response_type": "code",
        "client_id": registration["client_id"],
        "redirect_uri": callback_url(),
        "state": state,
        "code_challenge": challenge,
        "code_challenge_method": "S256",
        "resource": discovery["resource"],
    }
    if scopes:
        query["scope"] = " ".join(scopes)
    return endpoint + ("&" if "?" in endpoint else "?") + urlencode(query)


async def exchange(discovery: dict, registration: dict, code: str, verifier: str, response_issuer: str) -> dict:
    if response_issuer and response_issuer.rstrip("/") != discovery["issuer"]:
        raise ValueError("OAuth authorization response issuer mismatch")
    endpoint = discovery["metadata"].get("token_endpoint")
    if not endpoint:
        raise ValueError("OAuth token endpoint is missing")
    body = {
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": callback_url(),
        "client_id": registration["client_id"],
        "code_verifier": verifier,
        "resource": discovery["resource"],
    }
    if registration.get("client_secret"):
        body["client_secret"] = registration["client_secret"]
    status, _, raw = await request_public("POST", endpoint, data=urlencode(body), headers={"Content-Type": "application/x-www-form-urlencoded"})
    if status < 200 or status >= 300:
        raise ValueError("OAuth token exchange failed")
    token = json.loads(raw)
    if not token.get("access_token"):
        raise ValueError("OAuth access token is missing")
    return {**token, "issuer": discovery["issuer"], "resource": discovery["resource"], "token_endpoint": endpoint, "obtained_at": int(time.time())}


async def refresh(token: dict, registration: dict) -> dict:
    if not token.get("refresh_token"):
        raise ValueError("OAuth reauthorization is required")
    body = {
        "grant_type": "refresh_token", "refresh_token": token["refresh_token"],
        "client_id": registration["client_id"], "resource": token["resource"],
    }
    if registration.get("client_secret"):
        body["client_secret"] = registration["client_secret"]
    status, _, raw = await request_public("POST", token["token_endpoint"], data=urlencode(body), headers={"Content-Type": "application/x-www-form-urlencoded"})
    if status < 200 or status >= 300:
        raise ValueError("OAuth refresh failed")
    updated = json.loads(raw)
    if not updated.get("access_token"):
        raise ValueError("OAuth refresh token response is invalid")
    return {**token, **updated, "refresh_token": updated.get("refresh_token", token["refresh_token"]), "obtained_at": int(time.time())}


def new_state() -> tuple[str, str]:
    return secrets.token_urlsafe(32), secrets.token_urlsafe(64)
