<div align="center">

# MIX-agent

**A self-hosted AI workspace for individuals and trusted LAN/VPN networks**

[![CI](https://github.com/taitai2661/MIX-agent/actions/workflows/ci.yml/badge.svg)](https://github.com/taitai2661/MIX-agent/actions/workflows/ci.yml)
[![Release](https://github.com/taitai2661/MIX-agent/actions/workflows/release.yml/badge.svg)](https://github.com/taitai2661/MIX-agent/actions/workflows/release.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Docker](https://img.shields.io/badge/docker--compose-2496ED?logo=docker&logoColor=white)](compose.yaml)

[日本語](README.md) | English

</div>

---

MIX-agent is a self-hosted AI workspace for running AI on your own machine or inside a network you trust (LAN/VPN). Built with React + FastAPI + PostgreSQL and started with Docker Compose, it keeps chat, long-running agent tasks, browser automation, MCP, and local file work fully under your own infrastructure.

> **Status**: Early development software. Do not expose it to the public internet, do not use it for multiple untrusted users, and do not use it to safely execute untrusted code.

## Highlights

- **Chat / Thinking / Agent modes**: SSE streaming, conversation history, and attachments. Agent mode adds an autonomous execution foundation with checkpoints, stagnation detection, verification loops, and dynamic budget adjustment
- **Wide provider support**: 52 presets covering OpenAI / Anthropic / Gemini, major compatible APIs, and local runtimes, plus custom connections — with automatic Tool Calling verification via Capability Probe
- **Tool Registry and permissions**: control built-in tools (Web / Files / Terminal / Browser / Memory) with Always Allow / Ask / Deny. Approvals are persisted
- **MCP support**: stdio / Streamable HTTP, OAuth flows for remote MCP servers, and a filesystem onboarding template
- **Projects / Knowledge / Skills**: project-scoped instructions, materials, and conversation organization; chunk-based text retrieval; import/export of `SKILL.md` (Agent Skills)
- **Operations and safety**: encrypted backups and restore, operational diagnostics, isolated runners with allowed-domain controls (see [Security boundaries](#security-boundaries))

## Editions

This repository hosts two distinct editions in a monorepo structure:

| Edition | Tech Stack | Characteristics & Use Cases | Installation & How to Run |
| --- | --- | --- | --- |
| **Desktop Edition** (`apps/desktop`) | Tauri 2 + Rust + React | Personal local-first, native UI, fast and lightweight | **Recommended**: Download `.dmg` etc. from Releases<br>Development: `pnpm desktop:dev` |
| **Docker Edition** (`apps/server`, `apps/web`) | FastAPI + PostgreSQL + React | Self-hosted, LAN/VPN sharing, long-running background tasks | `docker compose up -d --build` |

## Installation

### Start with Desktop App (Native Application - Recommended)

#### 1. Install via Pre-built Package (Recommended)

Download the installer for your operating system from [GitHub Releases](https://github.com/taitai2661/MIX-agent/releases):

- **macOS**:
  - Apple Silicon (M1/M2/M3/M4): `mix-agent-desktop_*_aarch64.dmg`
  - Intel Mac: `mix-agent-desktop_*_x64.dmg`
  - Open the downloaded `.dmg` and drag & drop `MIX-agent Desktop` into your `Applications` folder.
  - *(For self-signed or unnotarized builds)* If macOS displays a gatekeeper warning on first launch, right-click (or Control-click) the application icon and select "Open", or run `xattr -cr "/Applications/MIX-agent Desktop.app"` in Terminal.
- **Windows**: `*.msi` or `*.exe` installer
- **Linux**: `*.deb` or `*.AppImage` package

#### 2. Run / Build from Source

Rust (1.82+), Node.js (22+), and pnpm are required.

```sh
# Install dependencies
pnpm install

# Start Desktop App (Tauri Dev)
pnpm desktop:dev

# Build local .dmg package for macOS
pnpm desktop:build:dmg
```

1. Add a provider and save the API key. The model list is fetched automatically, and models whose context window can be verified are added to the Auto candidates.
2. Re-run "connection test" or "fetch models" as needed.
3. Review each model's capabilities. Manually override anything unknown.
4. Set the default model, tool permissions, and the domains runners may communicate with.
5. Add agents / MCP servers if needed.

For local Ollama / LM Studio, explicitly allow private connections in the provider settings.

### Start with Docker (Self-hosted Server)

Docker Desktop, or Docker Engine with Compose v2, is required.
Simply clone the repository and run Compose at the root—**no extra setup or environment file required**:

```sh
git clone https://github.com/taitai2661/MIX-agent.git
cd MIX-agent
docker compose up -d --build
```

Open [http://localhost:8080](http://localhost:8080) and create the administrator. No environment variables or pre-provisioned API keys are required. localhost inside Docker is not your host machine; use `host.docker.internal` in the default URL when connecting to local Ollama / LM Studio.

### Connect a project folder

Mount the repository you want to work on read-only at `/project`. Its `AGENTS.md` (or `CLAUDE.md`) is loaded into every run's instructions, and the first existing `SKILL.md` among `.mix/skills` → `.claude/skills` → `.agents/skills` is provided as a skill. None of this is stored in the database; edit the files and they take effect from the next run.

```sh
MIX_PROJECT_DIR=/path/to/your/repo docker compose up -d
```

Without `MIX_PROJECT_DIR`, the directory containing the Compose file is mounted. If there are no instruction files or skills, nothing is added. Scheduled runs do not receive project context (instructions and materials) — only interactive runs do.

### Update and back up

Take an encrypted backup from the admin screen before updating, and keep the password required to restore. Pulling the code and rebuilding applies DB migrations on startup.

```sh
git pull --ff-only
docker compose up -d --build
```

State is stored in Docker volumes. `docker compose down` stops the containers, but `docker compose down -v` also deletes the volumes — do not run it when updating. Moving from a manual Compose setup to the ZimaOS store version does not necessarily carry volumes over automatically. See the [ZimaOS guide](docs/zimaos.md) for backup and restore procedures.

### Releases

[GitHub Releases](https://github.com/taitai2661/MIX-agent/releases) distributes all artifacts alongside per-version changelogs:
- **Desktop Installers**: macOS `.dmg` (Apple Silicon / Intel), Windows `.msi` / `.exe`, Linux `.deb` / `.AppImage`, and checksum manifest (`SHA256SUMS`)
- **Docker Container Images**: Published to GitHub Container Registry (GHCR)
- **ZimaOS / CasaOS Store Archives**: `mix-agent-zimaos-store-*.tar.gz`

The latest version is [v0.3.5](docs/release-notes-v0.3.5.md). The publishing procedure is summarized in the [release checklist](docs/release-checklist.md).

## Features

- Chat / Thinking / Agent, SSE streaming, conversation history, attachments
- Projects (dedicated instructions, material text, conversation organization) and research mode using Agent (web research with source-cited reports)
- 52 provider presets (OpenAI, Anthropic, Gemini, major compatible APIs, Japanese and local runtimes) and custom connections for OpenAI-compatible / Anthropic / Gemini
- Common Tool Registry, Always Allow / Ask / Deny, persisted approvals
- Web / Files / Terminal / Browser / Memory, user-facing plans
- stdio / Streamable HTTP MCP, filesystem onboarding template
- Agent editing, memory history and restore, encrypted secrets
- Encrypted backups, pre-restore evacuation, recovery from interrupted restores
- Operational diagnostics (DB connectivity, runner settings, stalled runs, recent failures and interruptions)
- Knowledge text registration and search tool, skill registration, history, restore, scheduled runs
- Agent Skills (`SKILL.md`) import/export (paste / zip / server directory), bundled-material reading tool
- OAuth flows for remote MCP servers (requires a compatible server)
- Conversation export to Markdown (answer only / full conversation / tool execution summary)

Knowledge provides chunk-based text search. Automatic sync of external materials and real-world evaluation of search quality remain future work. Image generation and editing, a registry store, and an externally facing OpenAI Compatible API are next-stage items. Image attachment and analysis with vision-capable models is included.

### Agent Skills

`SKILL.md` files (YAML frontmatter + Markdown) with required `name` and `description` can be imported. Paste the body on the Skills screen, upload a `.zip`, or place it at `/data/skills/<name>/SKILL.md` in the data volume and run "import from server directory" (`MIX_SKILLS_DIR` changes the location). Each skill can be exported as `SKILL.md` or `.zip`. Bundled files under `scripts` / `references` / `assets` are stored; the AI reads them with the `skill_resource` tool but never executes scripts. `allowed-tools` is display-only and does not auto-approve permissions.

Project materials are currently passed to each run as pasted text. Existing conversations can be moved into a project from "manage conversations". Deleting a project does not delete its conversations. Research mode requires an active web search or browser tool and a tool-calling-capable model. Verify the accuracy of sources yourself.

When the context limit approaches, older conversations are summarized before being sent to the model. The chat screen shows the compressed summary and the number of items per run. If a summary cannot be generated, the run stops instead of silently dropping old context. A run's "answer check" only judges whether a final user-facing answer exists; it does not guarantee correctness or task completion.

### Chat / Thinking / Agent

- **Chat**: on verified models, uses thinking, search, creation, and execution as needed. Normal conversation works even when thinking or Tool Calling is unsupported or unverified.
- **Thinking**: performs search, creation, and execution as needed while using standard reasoning. Provider-specific thinking is enabled for reasoning-capable models, but Thinking also works on unsupported models. Search on every turn or the display length of thinking text is not guaranteed.
- **Agent**: keeps the existing thinking settings and long-work execution loop, adding periodic checkpoints, stagnation detection with self-repair guidance, strict verification loops, hierarchical context compression, and dynamic budget adjustment.

Built-in tools are provided in all modes by default. When a preset is selected, its tool list (including an empty list) takes precedence and MCP is not added automatically. Ask / Deny, approvals, and runner isolation apply regardless of mode. Limits are 20 minutes / 12 steps / 12 tool calls for Chat, 45 minutes / 24 steps / 24 tool calls for Thinking, and 90 minutes / 300 steps / 750 tool calls for Agent. Use Agent for long work.

#### Agent autonomy

- **Intermediate checkpoints**: every 10 steps, history, task state, plan, and recent tool calls are saved to `run_checkpoints`. After an interruption you can resume from any checkpoint via `from_checkpoint` on `POST /runs/{key}/resume`.
- **Stagnation detection and self-repair**: repeated failures, read stalls, and provider instability are detected from the last 12 tool calls, and repair guidance is injected into the user-role history each step. Shown in the UI as a `stagnation_detected` event.
- **Strict verification loops**: verification is required at each `phase:<name>` boundary in `update_plan`. `files_list` / `search_files` after `delete_file`, `read_file` before overwriting with `write_file`, and a check tool after destructive terminal operations are mandatory.
- **Hierarchical context compression**: active summaries (up to 4000 characters) and archive rows (up to 12) are managed separately. Task state (goal / plan / pending / important_facts / artifacts / open_questions) is preserved first in the head block.
- **Dynamic budget adjustment**: when step or tool limits are reached, the user is asked to extend (2 times by default). `POST /runs/{key}/budget-extension` accepts `grant: true/false`, so runs do not stop without confirmation.
- **Selective handling of unknown results**: the resume API's `unknown_actions` controls each tool call left `executing` on a `retry` / `discard` / `mark_unknown` basis.

Provider-specific thinking control for Chat / Thinking supports OpenAI Responses, OpenRouter, identifiable Claude 3.7 / 4 series, and Gemini 2.5 / 3 series. On Ollama / LM Studio / generic compatible APIs and unidentified models, Thinking runs with standard reasoning without sending dedicated parameters. When a model's Tool Calling is unverified, all three modes automatically verify it with side-effect-free dummy functions before running. Results are stored, and runs continue without tools when unverified or unsupported. Re-verification follows the capability probe freshness policy (6 hours when unresolved, 14 days when confirmed) and also runs on scheduler ticks. Manual re-run is only available via "re-verify Tool Calling" in the model settings; manual overrides take precedence over automatic verification.

The API `mode` is unchanged. In `GET /models`, each record's `data.reasoning_control` is the computed control mode or null, and `data.tool_probe` stores the saved Tool Calling verification result. `POST /models/{id}/verify-tools` runs a side-effect-free re-verification. Runs store the resolved thinking policy and tool list; older runs without the extra information behave as before. Only published thinking summaries are displayed, and signed context is retained for tool continuation.

### Run Events

The SSE stream at `GET /runs/{key}/events` emits the following `kind` values (the Engine→UI representation contract):

- Existing: `text` / `reasoning` / `model_started` / `model_selected` / `model_rerouted` / `message` / `status` / `tool_started` / `tool_result` / `plan` / `approval` / `browser_frame` / `context_summary`
- New: `checkpoint_saved` / `budget_extension_requested` / `budget_extension_resolved` / `stagnation_detected` / `verification_required` / `phase_advanced`

Checkpoints in `run_checkpoints` are listed via `GET /runs/{key}/checkpoints`. For runs in `budget_extension_pending`, `POST /runs/{key}/budget-extension` accepts `grant` and an optional budget override.

Control spec references: [OpenAI](https://developers.openai.com/api/docs/guides/reasoning), [Claude](https://platform.claude.com/docs/en/build-with-claude/adaptive-thinking), [Gemini GenerateContent](https://ai.google.dev/gemini-api/docs/generate-content/thinking), [OpenRouter](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens). See [verification status](docs/verification.md) for the scope confirmed against real providers.

## Documentation

| Document | Contents |
| --- | --- |
| [Architecture](docs/architecture.md) | Service layout and design |
| [Operations](docs/operations.md) | Operational procedures |
| [Verification status](docs/verification.md) | What has been verified in real environments |
| [OpenAPI spec](docs/openapi.json) | API spec (generated TypeScript types at `apps/web/src/generated/api.ts`) |
| [ZimaOS guide](docs/zimaos.md) | Installing via the ZimaOS Community Store |
| [Contributing](CONTRIBUTING.md) | Development workflow and pull requests |

## Security boundaries

- By default, only localhost:8080 is exposed. Avoid LAN exposure before the initial setup is complete.
- The terminal can operate on the entire dedicated workspace; there is no strong isolation between subfolders.
- Runners do not mount the host, the Docker socket, provider keys, or DB passwords.
- Terminal and MCP run in separate containers. MCP servers share a single administrator's trust domain.
- Runner traffic goes through the internal network and an egress proxy. With no allowed domains configured, public domains are allowed; when configured, they are treated as an exact-match allowlist. Private IPs are rejected.
- If arbitrary shell code is allowed, there is no guarantee that package installation alone can be completely blocked.
- Installing MCP packages or changing allowed destinations is administrator-only. The AI cannot change these settings.
- Docker isolation is not a substitute for dedicated OS/VM isolation of adversarial code.
- Messages, images, and tool results sent to a provider are subject to that provider's data terms.
- Presets only configure connection methods and default URLs. Check actual model support, pricing, and terms with each provider. Presets with an empty URL connect via the base URL of your organization or local environment.

## Development

Node.js 22+ and pnpm are required. The application database is PostgreSQL-only, and everything starts via Docker Compose.

Supplementary model data (context lengths, pricing, capabilities) is fetched from [models.dev](https://models.dev)'s `api.json`. The default source is `https://models.dev/api.json`; set `MIX_MODELS_DEV_URL` to another URL or a local `api.json` path to override it. If it cannot be fetched, the supplementary data for that model is left unset. The old `MIX_MODEL_INFO_URL` is deprecated.

```sh
# Build the frontend
cd apps/web
pnpm install --frozen-lockfile
pnpm build
cd ../..
docker compose up -d --build

# Tests (from the repository root; runs migrations on a throwaway PostgreSQL)
docker compose --profile test run --rm test
cd apps/web
pnpm test
pnpm build
```

App startup, including the database, is fully handled by Compose; there is no host-shell runner fallback. The API spec lives in `docs/openapi.json` and the generated TypeScript types in `apps/web/src/generated/api.ts`. Update them with `scripts/export-openapi.py` and `pnpm generate:api`.

See [CONTRIBUTING.md](CONTRIBUTING.md), [architecture](docs/architecture.md), [operations](docs/operations.md), and [verification status](docs/verification.md) for details.

## License

This project is licensed under the [Apache License 2.0](LICENSE). See [NOTICE](NOTICE) for details. By contributing, you agree that your contributions will be licensed under the same Apache License 2.0.
