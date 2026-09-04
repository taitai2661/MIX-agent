import os
from pathlib import Path
import pytest

ROOT = Path("/tmp/mix-tests")
os.environ["MIX_DATA"] = str(ROOT)
os.environ["MIX_KEYS"] = str(ROOT / "keys")
os.environ["DATABASE_URL"] = "postgresql+psycopg://mix:test-only-password@postgres-test/mix"
os.environ["PUBLIC_ORIGIN"] = "http://testserver"

from mix_agent.db.session import engine, SessionLocal
from mix_agent.db.models import Base
from mix_agent.main import app
from fastapi.testclient import TestClient


@pytest.fixture(autouse=True)
def database():
    Base.metadata.drop_all(engine)
    Base.metadata.create_all(engine)
    yield


@pytest.fixture
def client():
    return TestClient(app)


@pytest.fixture
def signed(client):
    response = client.post(
        "/api/v1/setup/admin",
        json={"username": "tester", "password": "test-password-12345"},
    )
    assert response.status_code == 200, response.text
    client.headers["x-csrf-token"] = response.json()["csrf"]
    return client
