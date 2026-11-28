import os
import shutil
import tempfile
from pathlib import Path

import pytest

# A unique root per pytest process avoids leaking keys/data between concurrent
# or repeated runs; the session fixture below removes it at the end.
ROOT = Path(tempfile.mkdtemp(prefix="mix-tests-"))
os.environ["MIX_DATA"] = str(ROOT)
os.environ["MIX_KEYS"] = str(ROOT / "keys")
# Never inherit DATABASE_URL from the developer shell: the `database` fixture
# below drops and recreates every table, so an exported staging/production URL
# would wipe a real database. Tests always use the disposable test service.
os.environ["DATABASE_URL"] = "postgresql+psycopg://mix@postgres-test/mix"
os.environ["PUBLIC_ORIGIN"] = "http://testserver"
# Point the models.dev catalog at a path that never exists so no test reaches
# the network; individual tests override the lookup itself when they need one.
os.environ["MIX_MODELS_DEV_URL"] = str(ROOT / "models-dev.json")

from fastapi.testclient import TestClient
from mix_agent.db.models import Base
from mix_agent.db.session import engine
from mix_agent.main import app


# Test isolation model (deliberately minimal, do not grow this file):
# - `database` drops/recreates every table per test: integration tests get a
#   fresh PostgreSQL schema and never see each other's rows.
# - `client`/`signed` build on `database`, so pure unit tests that need no DB
#   must NOT take these fixtures — they run without Docker (see CI
#   `docker-backend` vs host-only unit runs). If this file gains more than
#   fixtures (helpers, factories), move them to tests/helpers/.
@pytest.fixture(scope="session", autouse=True)
def _clean_test_root():
    yield
    shutil.rmtree(ROOT, ignore_errors=True)


@pytest.fixture
def database():
    # Only tests that exercise the API (client/signed) require PostgreSQL;
    # pure unit tests run without Docker.
    Base.metadata.drop_all(engine)
    Base.metadata.create_all(engine)
    yield


@pytest.fixture
def client(database):
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
