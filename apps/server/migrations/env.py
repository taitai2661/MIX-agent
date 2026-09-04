from alembic import context
from mix_agent.db.session import engine
from mix_agent.db.models import Base

with engine.connect() as connection:
    context.configure(connection=connection, target_metadata=Base.metadata)
    with context.begin_transaction():
        context.run_migrations()
