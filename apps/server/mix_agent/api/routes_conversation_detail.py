"""Point reads for one conversation.

The list endpoint in ``routes_conversations`` is paginated. The chat header
still needs its own row when the thread falls outside the current page, and
fetching the whole page just to find one title was the old workaround.
"""

from fastapi import APIRouter, Depends

from mix_agent.api.common import context, own, public
from mix_agent.api.schemas import RecordView
from mix_agent.db.models import Conversation

router = APIRouter(prefix="/api/v1")


@router.get("/conversations/{key}", response_model=RecordView)
def get_conversation(key: str, ctx=Depends(context)):
    db, s = ctx
    return public(own(db, Conversation, key, s.owner_id))
