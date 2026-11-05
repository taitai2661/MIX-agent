FROM node:22.16.0-bookworm-slim@sha256:048ed02c5fd52e86fda6fbd2f6a76cf0d4492fd6c6fee9e2c463ed5108da0e34 AS web
WORKDIR /web
RUN corepack enable && corepack prepare pnpm@10.11.0 --activate
COPY apps/web/package.json apps/web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY apps/web/ ./
RUN pnpm build

FROM python:3.15.0-slim-bookworm@sha256:0e019bbdfb5d9874cb2389f1c2884b64aa2ac97c99960b5bc521cef978c0617d AS runtime
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1
WORKDIR /app
COPY LICENSE NOTICE /app/
COPY apps/server/pyproject.toml apps/server/uv.lock /app/
RUN pip install --no-cache-dir uv==0.12.7 && uv sync --frozen --no-dev --no-install-project
ENV PATH="/app/.venv/bin:$PATH"
RUN useradd -u 1000 -m mix

FROM python:3.15.0-slim-bookworm@sha256:0e019bbdfb5d9874cb2389f1c2884b64aa2ac97c99960b5bc521cef978c0617d AS init
COPY deploy/docker/bootstrap.py /bootstrap.py
ENTRYPOINT ["python", "/bootstrap.py"]

FROM runtime AS app
COPY apps/server/ /app/
COPY --from=web /web/dist /web
ENV WEB_DIST=/web MIX_DATA=/data MIX_KEYS=/keys PYTHONPATH=/app
USER 1000:1000
CMD ["uvicorn", "mix_agent.main:app", "--host", "0.0.0.0", "--port", "8080", "--workers", "1"]

FROM runtime AS test
RUN uv sync --frozen --dev --no-install-project
COPY apps/server/ /app/
COPY tests/ /workspace/tests/
COPY runners/ /workspace/runners/
COPY packages/ /workspace/packages/
COPY deploy/ /workspace/deploy/
# The contract test compares memory/jobs.py against docs/memory-spec.md §6, so
# the spec has to travel with the test image (scripts/check-memory-parity.py
# runs the same comparison on the host and in CI).
COPY docs/ /workspace/docs/
# The retrieval bench is shipped with the test image so §5's ranking can be
# measured with one command in the same environment the contract tests use
# (PostgreSQL, no network, no LLM API): experiments/run_retrieval_bench.py.
COPY experiments/ /workspace/experiments/
# `tests/unit/test_repo_config.py` guards the repository's own tooling config.
# It runs inside this image, so the files it reads travel with the tests
# instead of being silently absent under /workspace.
COPY pytest.ini ruff.toml compose.yaml /workspace/
COPY apps/server/pyproject.toml /workspace/apps/server/pyproject.toml
COPY .github/workflows/ci.yml /workspace/.github/workflows/ci.yml
WORKDIR /workspace
ENV PYTHONPATH=/app:/workspace
USER 1000:1000

FROM runtime AS execution
USER root
RUN apt-get update && apt-get install -y --no-install-recommends git && rm -rf /var/lib/apt/lists/*
COPY runners/execution/main.py /app/runner.py
COPY packages/tool_contracts/snapshots.py /app/snapshots.py
USER 1000:1000
CMD ["uvicorn", "runner:app", "--host", "0.0.0.0", "--port", "8001"]

FROM runtime AS browser-base
USER root
# Runtime libraries only; Chromium itself is downloaded after setup into a volume.
RUN playwright install-deps chromium && rm -rf /var/lib/apt/lists/*
COPY runners/execution/main.py /app/runner.py
COPY runners/browser_provisioner/main.py /app/browser_provisioner.py
ENV PLAYWRIGHT_BROWSERS_PATH=/browser

FROM browser-base AS browser-runner
USER 1000:1000
CMD ["uvicorn", "runner:app", "--host", "0.0.0.0", "--port", "8005"]

FROM browser-base AS browser-provisioner
USER 1000:1000
CMD ["uvicorn", "browser_provisioner:app", "--host", "0.0.0.0", "--port", "8006"]

FROM runtime AS mcp
COPY --from=web /usr/local/bin/node /usr/local/bin/node
COPY --from=web /usr/local/lib/node_modules/npm /usr/local/lib/node_modules/npm
RUN ln -s /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm
COPY runners/mcp/main.py /app/mcp_runner.py
COPY apps/server/mix_agent/mcp/protocol.py /app/mcp_protocol.py
COPY apps/server/mix_agent/tools/network.py /app/mix_agent/tools/network.py
COPY packages/tool_contracts/snapshots.py /app/snapshots.py
USER 1000:1000
CMD ["uvicorn", "mcp_runner:app", "--host", "0.0.0.0", "--port", "8002"]

FROM runtime AS mcp-runtime
USER root
COPY --from=web /usr/local/bin/node /usr/local/bin/node
COPY --from=web /usr/local/lib/node_modules/npm /usr/local/lib/node_modules/npm
RUN ln -s /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm
COPY runners/mcp_workload/main.py /app/mcp_workload.py
COPY runners/mcp_manager/main.py /app/mcp_manager.py
USER 1000:1000
CMD ["uvicorn", "mcp_workload:app", "--host", "0.0.0.0", "--port", "8004"]

FROM runtime AS egress
COPY apps/server/mix_agent/ /app/mix_agent/
COPY deploy/egress/proxy.py /app/proxy.py
USER 1000:1000
CMD ["python", "/app/proxy.py"]