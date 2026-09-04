import json

import pytest

from mix_agent.mcp.protocol import validate_schema
from mix_agent.mcp.registry import normalize


def test_registry_prefers_remote_then_oci_npm_pypi():
    manifest = normalize({
        "name": "io.example/test", "title": "Test", "description": "safe", "version": "1.2.3",
        "remotes": [{"transport": {"type": "streamable-http", "url": "https://mcp.example.com/mcp"}}],
        "packages": [
            {"registryType": "pypi", "identifier": "example-mcp", "version": "1.2.3", "transport": {"type": "stdio"}},
            {"registryType": "npm", "identifier": "@example/mcp", "version": "1.2.3", "transport": {"type": "stdio"}},
            {"registryType": "oci", "identifier": "ghcr.io/example/mcp:1.2.3", "version": "1.2.3", "transport": {"type": "stdio"}},
        ],
    })
    assert manifest["selected"]["kind"] == "remote"
    assert [row["kind"] for row in manifest["candidates"]] == ["remote", "oci", "npm", "pypi"]


def test_registry_rejects_ranges_and_custom_package_registries():
    with pytest.raises(ValueError):
        normalize({"name": "io.example/test", "version": "latest", "packages": []})
    manifest = normalize({
        "name": "io.example/test", "version": "1.0.0",
        "packages": [{"registryType": "npm", "registryBaseUrl": "https://evil.example", "identifier": "bad", "version": "1.0.0", "transport": {"type": "stdio"}}],
    })
    assert manifest["selected"] is None


def test_external_schema_refs_and_excessive_depth_are_rejected():
    with pytest.raises(ValueError):
        validate_schema({"type": "object", "$ref": "https://evil.example/schema.json"})
    value = {"type": "object"}
    current = value
    for _ in range(40):
        current["properties"] = {"next": {"type": "object"}}
        current = current["properties"]["next"]
    with pytest.raises(ValueError):
        validate_schema(value)

