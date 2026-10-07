"""Memory taxonomy: roles, scopes, lifecycle states, evidence kinds.

This is the canonical vocabulary for the role-based memory system.  All other
modules import from here so adding a new role / scope / lifecycle value only
touches one place.
"""

from __future__ import annotations

from typing import Final

# ---------------------------------------------------------------------------
# Role: what kind of knowledge the item holds.
# ---------------------------------------------------------------------------
ROLE_FACT: Final = "fact"
ROLE_DECISION: Final = "decision"
ROLE_PREFERENCE: Final = "preference"
ROLE_GOAL: Final = "goal"
ROLE_CONSTRAINT: Final = "constraint"
ROLE_EXPERIENCE: Final = "experience"
ROLE_FAILURE: Final = "failure"
ROLE_SOLUTION: Final = "solution"
ROLE_OBSERVATION: Final = "observation"
ROLE_HYPOTHESIS: Final = "hypothesis"
ROLE_QUESTION: Final = "question"
# Catch-all for items whose role cannot be classified into a known bucket.
# ``group_for_context`` funnels these into the "other" bucket so the context
# engine never silently drops them.
ROLE_UNKNOWN: Final = "unknown"

ROLES: Final[tuple[str, ...]] = (
    ROLE_FACT,
    ROLE_DECISION,
    ROLE_PREFERENCE,
    ROLE_GOAL,
    ROLE_CONSTRAINT,
    ROLE_EXPERIENCE,
    ROLE_FAILURE,
    ROLE_SOLUTION,
    ROLE_OBSERVATION,
    ROLE_HYPOTHESIS,
    ROLE_QUESTION,
    ROLE_UNKNOWN,
)

# Roles that describe *what the agent believes to be true right now* (semantic).
SEMANTIC_ROLES: Final = frozenset({ROLE_FACT, ROLE_DECISION, ROLE_PREFERENCE, ROLE_GOAL, ROLE_CONSTRAINT, ROLE_SOLUTION})
# Roles that describe *what happened* (episodic).
EPISODIC_ROLES: Final = frozenset({ROLE_EXPERIENCE, ROLE_FAILURE, ROLE_OBSERVATION})
# Roles that are not yet stable enough to drive action without confirmation.
TENTATIVE_ROLES: Final = frozenset({ROLE_HYPOTHESIS, ROLE_QUESTION})


# ---------------------------------------------------------------------------
# Scope: where the knowledge belongs.
# ---------------------------------------------------------------------------
SCOPE_WORKING: Final = "working"
SCOPE_TASK: Final = "task"
SCOPE_PROJECT: Final = "project"
SCOPE_USER: Final = "user"
SCOPE_WORLD: Final = "world"

SCOPES: Final[tuple[str, ...]] = (
    SCOPE_WORKING,
    SCOPE_TASK,
    SCOPE_PROJECT,
    SCOPE_USER,
    SCOPE_WORLD,
)


# ---------------------------------------------------------------------------
# Source: where the knowledge originally came from.  Distinct from evidence:
# source is a coarse tag for the trust profile; evidence is the actual citation.
# ---------------------------------------------------------------------------
SOURCE_USER: Final = "user"
SOURCE_AGENT: Final = "agent"
SOURCE_TOOL: Final = "tool"
SOURCE_BROWSER: Final = "browser"
SOURCE_EXTERNAL: Final = "external"
SOURCE_CONSOLIDATION: Final = "consolidation"

SOURCES: Final[tuple[str, ...]] = (
    SOURCE_USER,
    SOURCE_AGENT,
    SOURCE_TOOL,
    SOURCE_BROWSER,
    SOURCE_EXTERNAL,
    SOURCE_CONSOLIDATION,
)


# ---------------------------------------------------------------------------
# Verification: human / system verdict on a memory item.
# ---------------------------------------------------------------------------
VERIFICATION_UNVERIFIED: Final = "unverified"
VERIFICATION_PENDING: Final = "pending"
VERIFICATION_VERIFIED: Final = "verified"
VERIFICATION_DISPUTED: Final = "disputed"
VERIFICATION_REJECTED: Final = "rejected"

VERIFICATIONS: Final[tuple[str, ...]] = (
    VERIFICATION_UNVERIFIED,
    VERIFICATION_PENDING,
    VERIFICATION_VERIFIED,
    VERIFICATION_DISPUTED,
    VERIFICATION_REJECTED,
)


# ---------------------------------------------------------------------------
# Lifecycle: how the item ages in the agent's mind.
# ---------------------------------------------------------------------------
LIFECYCLE_CANDIDATE: Final = "candidate"
LIFECYCLE_ACTIVE: Final = "active"
LIFECYCLE_SUPERSEDED: Final = "superseded"
LIFECYCLE_EXPIRED: Final = "expired"
LIFECYCLE_ARCHIVED: Final = "archived"
LIFECYCLE_DISPUTED: Final = "disputed"

# Map old associative-memory states to the new vocabulary so legacy rows still
# make sense after migration.
LEGACY_LIFECYCLE_MAP: Final[dict[str, str]] = {
    "established": LIFECYCLE_ACTIVE,
    "latent": LIFECYCLE_CANDIDATE,
    "superseded": LIFECYCLE_SUPERSEDED,
    "archived": LIFECYCLE_ARCHIVED,
    "deleted": LIFECYCLE_EXPIRED,
}

LIFECYCLES: Final[tuple[str, ...]] = (
    LIFECYCLE_CANDIDATE,
    LIFECYCLE_ACTIVE,
    LIFECYCLE_SUPERSEDED,
    LIFECYCLE_EXPIRED,
    LIFECYCLE_ARCHIVED,
    LIFECYCLE_DISPUTED,
)

# Lifecycle states that may appear in retrieval results by default.
LIFECYCLE_RETRIEVABLE: Final = frozenset({LIFECYCLE_CANDIDATE, LIFECYCLE_ACTIVE})


# ---------------------------------------------------------------------------
# Evidence kinds.
# ---------------------------------------------------------------------------
EVIDENCE_USER_MESSAGE: Final = "user_message"
EVIDENCE_CONVERSATION: Final = "conversation"
EVIDENCE_TOOL_RESULT: Final = "tool_result"
EVIDENCE_BROWSER: Final = "browser"
EVIDENCE_FILE: Final = "file"
EVIDENCE_CODE_CHANGE: Final = "code_change"
EVIDENCE_EXTERNAL: Final = "external"
EVIDENCE_AGENT_ACTION: Final = "agent_action"
EVIDENCE_OBSERVATION: Final = "observation"

EVIDENCE_KINDS: Final[tuple[str, ...]] = (
    EVIDENCE_USER_MESSAGE,
    EVIDENCE_CONVERSATION,
    EVIDENCE_TOOL_RESULT,
    EVIDENCE_BROWSER,
    EVIDENCE_FILE,
    EVIDENCE_CODE_CHANGE,
    EVIDENCE_EXTERNAL,
    EVIDENCE_AGENT_ACTION,
    EVIDENCE_OBSERVATION,
)


# ---------------------------------------------------------------------------
# Relationship kinds between Memory items.  Stored on MemoryAssociation.data.
# ---------------------------------------------------------------------------
REL_SUPPORTS: Final = "supports"
REL_CONTRADICTS: Final = "contradicts"
REL_DERIVED_FROM: Final = "derived_from"
REL_SUPERSEDES: Final = "supersedes"
REL_CAUSED_BY: Final = "caused_by"
REL_RELATES_TO: Final = "relates_to"

RELATION_KINDS: Final[tuple[str, ...]] = (
    REL_SUPPORTS,
    REL_CONTRADICTS,
    REL_DERIVED_FROM,
    REL_SUPERSEDES,
    REL_CAUSED_BY,
    REL_RELATES_TO,
)
