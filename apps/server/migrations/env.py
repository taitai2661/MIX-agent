from alembic import context

from mix_agent.db.models import Base
from mix_agent.db.session import engine

# PostgreSQL-only indexes created with raw DDL in migrations 0001/0011/0022/
# 0026/0027: expression, trigram and GIN indexes that autogenerate cannot
# reproduce faithfully from SQLAlchemy metadata. They are real production
# indexes, not drift. Excluding them keeps `alembic check` meaningful for
# tables, columns and the regular indexes declared in the models.
#
# When adding or removing one of these, update this list and the migration
# together. A plain (non-expression) index belongs in `db/models.py` instead.
RAW_SQL_INDEXES = frozenset(
    {
        "agents_owner_model",
        "conversations_owner_deleted_archived",
        "conversations_owner_folder",
        "conversations_owner_last_message",
        "conversations_owner_project",
        "conversations_owner_selection_model",
        "knowledge_text_trgm",
        "memory_content_trgm",
        "messages_content_trgm",
        "models_owner_provider",
    }
)


def include_object(obj, name, type_, reflected, compare_to):
    if type_ == "index" and name in RAW_SQL_INDEXES:
        return False
    return True


with engine.connect() as connection:
    context.configure(
        connection=connection,
        target_metadata=Base.metadata,
        include_object=include_object,
    )
    with context.begin_transaction():
        context.run_migrations()
