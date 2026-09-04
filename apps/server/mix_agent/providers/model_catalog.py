"""Conservative context-window metadata for provider model listings.

Many ``/models`` endpoints only return an identifier.  This catalog fills in
well-known models so Auto can make a bounded choice; unknown identifiers stay
unknown rather than receiving a guessed limit.
"""

from __future__ import annotations


# Keep entries scoped to a provider wire format where possible.  Prefixes also
# cover dated snapshot identifiers published by the same provider.
_PREFIXES: dict[str, tuple[tuple[str, int], ...]] = {
    "openai": (
        ("gpt-4.1", 1_047_576),
        ("gpt-4o", 128_000),
        ("gpt-4-turbo", 128_000),
        ("gpt-4-", 8_192),
        ("o3", 200_000),
        ("o4-mini", 200_000),
        ("o1", 200_000),
    ),
    "anthropic": (
        ("claude-3", 200_000),
        ("claude-sonnet-4", 200_000),
        ("claude-opus-4", 200_000),
    ),
    "gemini": (
        ("gemini-2", 1_048_576),
        ("gemini-1.5", 1_048_576),
    ),
}


def context_window(kind: str, model_id: str) -> int | None:
    """Return a documented context limit for a recognized model ID only."""
    normalized = model_id.lower().removeprefix("models/")
    candidates = (normalized, normalized.rsplit("/", 1)[-1])
    kinds = (kind,) if kind in _PREFIXES else ("openai", "anthropic", "gemini")
    for candidate in candidates:
        for catalog_kind in kinds:
            for prefix, limit in _PREFIXES.get(catalog_kind, ()):
                if candidate.startswith(prefix):
                    return limit
    return None
