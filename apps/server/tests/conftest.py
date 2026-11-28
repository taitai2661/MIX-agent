"""Bootstrap for tests kept under apps/server.

The repository-root conftest is only discovered for the root ``tests/`` tree,
so set the minimum config env vars needed to import ``mix_agent.config`` here.
"""

import os

# Do not inherit a real DATABASE_URL: tests in this tree may drop/create tables.
os.environ["DATABASE_URL"] = "postgresql+psycopg://mix@postgres-test/mix"
os.environ.setdefault("PUBLIC_ORIGIN", "http://testserver")
