"""Upgrade a previous release's schema with data in it.

The regular suite builds tables from ``Base.metadata`` (``tests/conftest.py``),
so it can never catch a migration that only works on an empty database.  This
test creates a scratch database, upgrades it to revision ``0020``, inserts the
core rows a real installation always has, and then upgrades all the way to the
current head.  It asserts both that the upgrade succeeds and that the data is
still readable afterwards.

The migration chain is deliberately re-runnable (each revision guards the
objects it adds), and ``0001`` creates the current model metadata, so this is
not a historical-schema simulation.  It *does* exercise the path a user takes
when they skip several releases: run the guarded chain against populated
tables instead of an empty database.
"""

from __future__ import annotations

import os
import subprocess
import sys
import uuid
from pathlib import Path

import psycopg
import pytest
from alembic.config import Config
from alembic.script import ScriptDirectory
from sqlalchemy.engine import make_url

BASE_REVISION = "0020"


def _server_dir() -> Path:
    override = os.environ.get("MIX_SERVER_DIR")
    if override:
        return Path(override)
    # Inside the test image the app is installed at /app; locally it is a
    # sibling of the repository's tests directory.
    if Path("/app/alembic.ini").exists():
        return Path("/app")
    return Path(__file__).resolve().parents[2] / "apps" / "server"


def _libpq_url(sqlalchemy_url: str, **changes: str) -> str:
    url = make_url(sqlalchemy_url)
    url = url.set(drivername="postgresql", **changes)
    return url.render_as_string(hide_password=False)


def _database_url(sqlalchemy_url: str, database: str) -> str:
    """Keep the ``+psycopg`` driver: mix_agent.config rejects plain postgresql."""
    url = make_url(sqlalchemy_url).set(database=database)
    return url.render_as_string(hide_password=False)


def _current_head(server_dir: Path) -> str:
    config = Config(str(server_dir / "alembic.ini"))
    config.set_main_option("script_location", str(server_dir / "migrations"))
    head = ScriptDirectory.from_config(config).get_current_head()
    assert head is not None
    return head


def _run_alembic(server_dir: Path, database_url: str, revision: str) -> None:
    result = subprocess.run(  # noqa: S603 - fixed argv, no shell, revision from migration scripts
        [
            sys.executable,
            "-m",
            "alembic",
            "-c",
            str(server_dir / "alembic.ini"),
            "upgrade",
            revision,
        ],
        cwd=server_dir,
        env={**os.environ, "DATABASE_URL": database_url},
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        raise AssertionError(
            f"alembic upgrade {revision} failed ({result.returncode}):\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )


def test_upgrade_preserves_rows_created_before_the_release():
    server_dir = _server_dir()
    if not (server_dir / "alembic.ini").exists():
        pytest.skip("server checkout is not available")

    database_url = os.environ["DATABASE_URL"]
    database = f"mix_upgrade_{uuid.uuid4().hex[:12]}"
    admin_url = _libpq_url(database_url, database="postgres")
    target_url = _database_url(database_url, database)
    target_libpq_url = _libpq_url(database_url, database=database)

    with psycopg.connect(admin_url, autocommit=True) as admin:
        admin.execute(f'DROP DATABASE IF EXISTS "{database}" WITH (FORCE)')
        admin.execute(f'CREATE DATABASE "{database}"')

    try:
        _run_alembic(server_dir, target_url, BASE_REVISION)

        with psycopg.connect(target_libpq_url) as connection:
            connection.execute(
                """
                INSERT INTO users (id, username, password_hash, singleton)
                VALUES ('u-upgrade', 'upgrade-user', 'not-a-real-hash', 1)
                """
            )
            connection.execute(
                """
                INSERT INTO conversations (id, owner_id, data, created_at)
                VALUES ('c-upgrade', 'u-upgrade',
                        '{"title": "before upgrade"}'::jsonb, now())
                """
            )
            connection.execute(
                """
                INSERT INTO messages (id, owner_id, data, created_at, conversation_id)
                VALUES ('m-upgrade', 'u-upgrade',
                        '{"role": "user", "content": "hello before upgrade"}'::jsonb,
                        now(), 'c-upgrade')
                """
            )
            connection.commit()

        _run_alembic(server_dir, target_url, "head")

        with psycopg.connect(target_libpq_url) as connection:
            version = connection.execute("SELECT version_num FROM alembic_version").fetchone()
            conversation = connection.execute(
                "SELECT data->>'title' FROM conversations WHERE id = 'c-upgrade'"
            ).fetchone()
            message = connection.execute(
                "SELECT data->>'content' FROM messages WHERE id = 'm-upgrade'"
            ).fetchone()
            memory_links = connection.execute("SELECT to_regclass('memory_links') IS NOT NULL").fetchone()

        assert version is not None and version[0] == _current_head(server_dir)
        assert conversation == ("before upgrade",)
        assert message == ("hello before upgrade",)
        assert memory_links == (True,)
    finally:
        with psycopg.connect(admin_url, autocommit=True) as admin:
            admin.execute(f'DROP DATABASE IF EXISTS "{database}" WITH (FORCE)')
