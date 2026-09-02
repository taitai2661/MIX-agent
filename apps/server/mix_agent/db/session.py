from sqlalchemy import create_engine
from sqlalchemy.orm import sessionmaker
from mix_agent import config

config.initialize()
engine = create_engine(
    config.DATABASE_URL,
    pool_pre_ping=True,
)


SessionLocal = sessionmaker(engine, expire_on_commit=False)


def get_db():
    with SessionLocal() as db:
        yield db
