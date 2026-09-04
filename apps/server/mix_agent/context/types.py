"""Phase 1 Context Engine: shared types for budget-aware context assembly."""

from __future__ import annotations

CONTEXT_VERSION = 1

# Category keys used by BudgetManager, BuiltContext and ContextTrace.
CATEGORIES = (
    "system",
    "task_state",
    "summary",
    "recent_conversation",
    "memory",
    "skills",
    "knowledge",
    "tools",
    "attachments",
)

# Initial flexible shares. Unused shares flow to recent_conversation.
DEFAULT_SHARES = {
    "system": 0.10,
    "task_state": 0.05,
    "summary": 0.10,
    "recent_conversation": 0.35,
    "memory": 0.10,
    "skills": 0.10,
    "knowledge": 0.10,
    "reserve": 0.10,
}

TRIGGER_TYPES = ("interactive", "scheduled", "resume", "webhook")

EMPTY_TASK_STATE = {
    "goal": "",
    "constraints": [],
    "plan": [],
    "completed": [],
    "pending": [],
    "important_facts": [],
    "artifacts": [],
    "open_questions": [],
}

TASK_STATE_FIELDS = tuple(EMPTY_TASK_STATE.keys())
TASK_STATE_LIST_FIELDS = {k for k in TASK_STATE_FIELDS if k != "goal"}


class ContextBudgetError(Exception):
    """Raised when even minimal context (system + current message + task state) exceeds budget."""
