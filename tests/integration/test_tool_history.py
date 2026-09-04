from sqlalchemy import select

from mix_agent.db.models import Conversation, Run, ToolCall, User
from mix_agent.db.session import SessionLocal


def test_conversation_tool_history_reports_failure_and_unknown_retry(signed):
    with SessionLocal() as db:
        owner = db.scalar(select(User.id))
        conversation = Conversation(owner_id=owner, data={"title": "history"})
        db.add(conversation)
        db.flush()
        run = Run(
            owner_id=owner,
            conversation_id=conversation.id,
            request_key="history-request-key",
            status="interrupted",
            data={"snapshot": {}, "history": []},
        )
        db.add(run)
        db.flush()
        db.add_all([
            ToolCall(
                owner_id=owner,
                run_id=run.id,
                status="completed",
                data={
                    "tool_id": "web_search", "name": "web_search",
                    "result": {"error": "Tool failed", "token": "must-not-leak"},
                },
            ),
            ToolCall(
                owner_id=owner,
                run_id=run.id,
                status="executing",
                data={"tool_id": "run_terminal", "name": "run_terminal"},
            ),
        ])
        db.commit()
        conversation_id = conversation.id

    response = signed.get(f"/api/v1/conversations/{conversation_id}/tool-calls")
    assert response.status_code == 200, response.text
    failed, unknown = response.json()
    assert failed["status"] == "failed"
    assert failed["failure"] == "Tool failed"
    assert failed["result"]["token"] == "[redacted]"
    assert unknown["status"] == "unknown"
    assert unknown["retry"]["available"] is True
