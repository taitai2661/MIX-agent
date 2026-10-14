from sqlalchemy import create_engine, event
from sqlalchemy.orm import Session, sessionmaker

from mix_agent import config

config.initialize()
engine = create_engine(
    config.DATABASE_URL,
    pool_pre_ping=True,
    pool_size=10,
    max_overflow=20,
)


SessionLocal = sessionmaker(engine, expire_on_commit=False)


@event.listens_for(Session, "after_commit")
def _notify_committed_work(session):
    from mix_agent.wakeups import committed
    committed(session)


@event.listens_for(Session, "after_rollback")
def _discard_uncommitted_work(session):
    from mix_agent.wakeups import rolled_back
    rolled_back(session)


def get_db():
    with SessionLocal() as db:
        yield db
