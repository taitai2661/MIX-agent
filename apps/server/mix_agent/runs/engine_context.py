import asyncio


from mix_agent.context import budget as context_budget
from mix_agent.context import head_blocks
from mix_agent.context import task_state as context_task_state
from mix_agent.context import tokens as context_tokens
from mix_agent.context import tools_selector as context_tools
from mix_agent.context.builder import select_recent
from mix_agent.context.summary import finalize as finalize_summary
from mix_agent.context.summary import merge_prompt as summary_merge_prompt
from mix_agent.context.summary import render as render_summary
from mix_agent.context.types import ContextBudgetError
from mix_agent.db.models import (
    Artifact,
)
from mix_agent.providers.adapters import (
    Adapter,
)


from mix_agent.runs.engine_support import emit, refresh_task_state, update


def _image_data_url(db, owner_id, artifact_id):
    """Load an image artifact as a data URL; return None when missing (degrade)."""
    try:
        row = db.get(Artifact, artifact_id)
        if not row or row.owner_id != owner_id:
            return None
        from mix_agent import config as _config

        raw = (_config.ARTIFACTS / artifact_id).read_bytes()
        import base64 as _b64

        mime = (row.data or {}).get("mime", "image/png")
        return "data:" + mime + ";base64," + _b64.b64encode(raw).decode()
    except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
        return None


def _resolve_history_for_provider(db, run, history):
    """Resolve image_refs at send time; strip ref pointers from transport.

    Missing files degrade to text (never crash); misses are recorded on the run.
    """
    resolved, missing = [], []
    for message in history or []:
        entry = dict(message)
        refs = entry.pop("image_refs", None) or []
        entry.pop("tool_ref", None)
        images = list(entry.get("images") or [])
        # Hoist any legacy inline base64 into refs would have happened at build;
        # here only resolve stored refs.
        for ref_id in refs:
            data_url = _image_data_url(db, run.owner_id, ref_id)
            if data_url:
                images.append(data_url)
            else:
                missing.append(ref_id)
        # Drop legacy inline base64 from transport beyond the newest image message.
        if images:
            entry["images"] = images
        else:
            entry.pop("images", None)
        resolved.append(entry)
    # Keep only the newest image-bearing message with actual images to avoid
    # resending past images every turn.
    last_with_images = max((i for i, m in enumerate(resolved) if m.get("images")), default=None)
    if last_with_images is not None:
        for i, m in enumerate(resolved):
            if m.get("images") and i != last_with_images:
                m = dict(m)
                m.pop("images", None)
                resolved[i] = m
    if missing:
        update(run, missing_image_refs=missing)
    return resolved


async def _maybe_compact_context(db, run, snapshot, provider, key):
    """Pre-send budget check with progressive compaction.

    Overflow order: tool raw already ref'd at write time; move old
    conversation to summary; system / current user / task state are kept last.
    Returns provider-ready history (refs resolved).
    """
    from mix_agent.context import tiered_summary as tiered
    from mix_agent.db.models import Model as _Model

    history = run.data.get("history") or []
    # Checkpoint: keep structured task state alive even for legacy runs.
    if not run.data.get("task_state"):
        first_user = next((m.get("content", "") for m in history if m.get("role") == "user"), "")
        update(run, task_state=context_task_state.ensure(None, first_user[:1000]))
    if history and history[0].get("role") == "system":
        head = refresh_task_state(history[0].get("content", ""), run.data["task_state"])
        if head != history[0].get("content"):
            history = [{**history[0], "content": head}, *history[1:]]
            update(run, history=history)
    if run.data.get("summary") is None:
        tiered.install_summary(run.data, "", 0)
    model_record = (
        db.get(_Model, snapshot.get("model_record_id")) if snapshot.get("model_record_id") else None
    )
    model_data = dict((model_record.data if model_record else {}) or {})
    window_info = context_budget.resolve_window(model_data, snapshot)
    snapshot["context_window_info"] = window_info
    # Recompute budget on every step so provider switching re-budgets.
    tool_cost = context_tools.schema_cost(snapshot.get("tools") or [], snapshot.get("model_id", ""))
    total = context_budget.input_budget(window_info, tool_schema_tokens=tool_cost)
    model_id = snapshot.get("model_id", "")
    # total already excludes tool schemas: compare messages-only (no double count).
    estimated = context_tokens.count_messages(history, model_id)
    if estimated <= total:
        _persist_trace(db, run, snapshot, history, estimated, total, tool_cost, [])
        return _resolve_history_for_provider(db, run, history)
    # Evict old conversation (keep head + newest); summarize evicted progressively.
    twelve_pct = max(2000, int(total * 0.12))
    recent_budget = max(1000, total - twelve_pct - tool_cost)
    # Head is history[0] (system block); compact only the conversation tail.
    head, tail = (history[:1], history[1:]) if history else ([], [])
    recent, evicted = select_recent(tail, recent_budget, model_id)
    summarized: list = []
    if evicted:
        previous = (run.data.get("summary") or {}).get("text", "")
        summary_text = ""
        try:
            emit(db, run.id, "context_summary", {"status": "started", "evicted": len(evicted)})
            db.commit()
            summary_input = summary_merge_prompt(previous, evicted)
            async with asyncio.timeout(90):
                async for event in Adapter(provider, key).stream(
                    snapshot["model_id"],
                    summary_input,
                    [],
                    "chat",
                    {"max_output_tokens": 2048, "_session_id": run.conversation_id},
                ):
                    if event["kind"] == "response":
                        summary_text = event["message"]["content"]
            summary_text = finalize_summary(summary_text)
            if summary_text:
                # Tiered summary: rotate the previous active text into the
                # archive so the next compaction can reuse it as evidence.
                new_covered = int((run.data.get("summary") or {}).get("covered_count", 0)) + len(evicted)
                tiered.archive_summary(run.data, summary_text, new_covered)
                summarized = [{"evicted": len(evicted), "tier": 0}]
                emit(
                    db,
                    run.id,
                    "context_summary",
                    {
                        "status": "completed",
                        "summary": summary_text,
                        "archive_rows": len(tiered.current_archive(run.data)),
                    },
                )
                # Rewrite head summary line progressively (no full re-summarization).
                head_text = head[0].get("content") if head else ""
                head_text = _replace_summary_block(head_text, summary_text)
                if head:
                    head = [{**head[0], "content": head_text}]
                db.commit()
        except ContextBudgetError:
            # Re-raise upstream so the caller can decide.  We still want to
            # preserve the existing summary text; no archive rotation.
            emit(db, run.id, "context_summary", {"status": "failed"})
            db.commit()
            raise
        except Exception:  # noqa: BLE001 - intentionally classified; never leak raw details
            # A summarizer failure must never cost the user their transcript.
            # Keep run.data["history"] exactly as it was, record why compaction
            # degraded, and let the next compaction opportunity retry. Continuing
            # with an over-budget history is recoverable; silently dropping the
            # evicted tail is not.
            emit(
                db,
                run.id,
                "context_summary",
                {
                    "status": "failed",
                    "evicted": len(evicted),
                    "preserved": True,
                },
            )
            update(
                run,
                context_compaction={
                    "status": "degraded",
                    "reason": "summary_failed",
                    "uncompacted_messages": len(history),
                    "covered_count": 0,
                },
            )
            db.commit()
            raise ContextBudgetError(
                "[history preserved] 要約の生成に失敗したため、古い会話を削除せず Context を"
                "圧縮できませんでした。履歴は保持されています。新しい会話で再開するか、"
                "Summary 対応が安定した Window サイズのモデルへ切り替えてください。"
            ) from None
        if not summary_text:
            emit(
                db,
                run.id,
                "context_summary",
                {
                    "status": "failed",
                    "evicted": len(evicted),
                    "preserved": True,
                },
            )
            update(
                run,
                context_compaction={
                    "status": "degraded",
                    "reason": "summary_empty",
                    "uncompacted_messages": len(history),
                    "covered_count": 0,
                },
            )
            db.commit()
            raise ContextBudgetError(
                "[history preserved] 要約が空のため、古い会話を削除せず Context を圧縮できませんでした"
                "（履歴は保持されています）。新しい会話で再開してください。"
            )
    compacted = [*head, *recent]
    estimated = context_tokens.count_messages(compacted, model_id)
    if estimated > total + 2000:
        raise ContextBudgetError(
            f"context budget exceeded: estimated {estimated} > {total} tokens "
            f"(window {window_info.get('context_window')})"
        )
    update(run, history=compacted)
    _persist_trace(db, run, snapshot, compacted, estimated, total, tool_cost, summarized)
    db.commit()
    return _resolve_history_for_provider(db, run, compacted)


def _replace_summary_block(head_text: str, summary_text: str) -> str:
    """Swap the Prior conversation summary block, keeping every other block.

    An empty summary removes the block. The blocks after the summary (Memory,
    Skills, Knowledge) are the ones an Agent needs most once a run gets long, so
    this must never truncate them.
    """
    return head_blocks.with_block(head_text, head_blocks.PRIOR_SUMMARY, render_summary(summary_text))


def _persist_trace(db, run, snapshot, history, estimated, total, tool_cost, summarized):
    model_id = snapshot.get("model_id", "")
    history = history or []
    # Per-category breakdown for token visibility: head (system block) vs
    # conversation tail vs tool schemas. Cheap to compute (head only) and
    # lets the dashboard/RunView show where input tokens actually go.
    head_text = ""
    if history and history[0].get("role") == "system":
        head_text = str(history[0].get("content") or "")
    head_tokens = context_tokens.count(head_text, model_id)
    conversation_tokens = max(0, estimated - head_tokens)
    window = (snapshot.get("context_window_info") or {}).get("context_window")
    trace = {
        "model": model_id,
        "trigger": run.data.get("trigger_type", "interactive"),
        "context_version": 1,
        "context_window": window,
        "input_budget": total,
        "estimated_input_tokens": estimated,
        "tool_schema_tokens": tool_cost,
        "head_tokens": head_tokens,
        "conversation_tokens": conversation_tokens,
        "budget_utilization": round(estimated / total, 3) if total else None,
        "history_messages": len(history),
        "steps": int(run.data.get("steps", 0)),
        "tool_count": int(run.data.get("tool_count", 0)),
        "summarized": summarized,
        "missing_image_refs": run.data.get("missing_image_refs", []),
        "task_state_present": bool(run.data.get("task_state")),
    }
    update(run, context_trace=trace)
