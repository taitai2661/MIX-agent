<div align="center">

# MIX-agent

**AI Workspace for Local & Self-Hosted Environments**

[![CI](https://github.com/taitai2661/MIX-agent/actions/workflows/ci.yml/badge.svg)](https://github.com/taitai2661/MIX-agent/actions/workflows/ci.yml)
[![Release](https://github.com/taitai2661/MIX-agent/actions/workflows/release.yml)](https://github.com/taitai2661/MIX-agent/actions/workflows/release.yml)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Docker](https://img.shields.io/badge/docker--compose-2496ED?logo=docker&logoColor=white)](compose.yaml)

[日本語](README.md) | English

</div>

---

MIX-agent is an AI workspace designed to run securely on your local machine or trusted private network (LAN/VPN) without relying on external cloud hosting.

This repository provides two distinct editions tailored for different use cases:

- **Desktop Edition**: A lightweight, fast, native desktop app (Tauri 2 + Rust + React) designed for everyday personal use on your local machine. No Docker required; install and launch directly from native installers.
- **Docker Edition**: Built specifically for **self-hosting private chat AI on home servers (HomeLab, NAS, Mini PCs)**. Runs 24/7 in the background with FastAPI + PostgreSQL + React, accessible from phones, tablets, and PCs via web browser over LAN or VPN.

> [!NOTE]
> **Development Status**: This is an early development version. Not intended for unrestricted public internet exposure, multi-tenant usage, or untrusted/adversarial code execution.

---

## Which Edition Should You Choose?

Choose the edition that best fits your environment and workflow:

| Feature / Aspect | Desktop Edition (`apps/desktop`) | Docker Edition (`apps/server`, `apps/web`) |
| :--- | :--- | :--- |
| **Primary Need & Use Case** | **Work immediately on your local PC (Recommended)**<br>Local coding, direct file editing, and developer workflows | **Self-host chat AI on a home server**<br>24/7 background uptime, LAN/VPN sharing, NAS (ZimaOS/CasaOS), HomeLab |
| **Client Access** | Installed local PC only | **Web Browser** (Access anywhere across phones, tablets, PCs) |
| **Uptime & Lifecycle** | Active only when your PC is running | **24/7 always-on background server** |
| **Setup Effort** | ⭐⭐⭐ **Effortless** (No Docker required)<br>Download installer from Releases and launch | ⭐⭐ **Standard** (Requires Docker Engine & Compose)<br>`git clone` and `docker compose up -d` |
| **User Interface** | OS Native Window, System Tray | Web Browser (Responsive UI) |
| **Tech Stack** | Tauri 2 + Rust + React 19 | FastAPI (Python) + PostgreSQL + React 19 |
| **Process Architecture** | Single binary + tokio in-proc (+ Chromium child process) | Docker Compose (app, web, db, runner, proxy containers) |
| **Storage & Secrets** | Local SQLite (WAL/FTS5) + **OS Keychain** | PostgreSQL + AES-GCM Encryption (Docker volumes) |
| **Terminal & Files** | Direct host file editing, **persistent PTY shell** | Operations isolated inside mounted `/project` or runner |
| **Browser Operator** | Bundled Chromium / direct system browser automation | Playwright inside dedicated `execution-runner` |
| **Security & Sandbox** | OS user permissions + **per-action scoped approval rules** | Container sandbox (non-root runner, egress-proxy rules) |

---

## Getting Started

### Desktop Edition (Native Local App — Recommended)

Get started right away without needing Docker or server setup.

#### 1. Install via Pre-built Binaries (Recommended)

Download the installer for your OS from [GitHub Releases](https://github.com/taitai2661/MIX-agent/releases):

- **macOS**:
  - Apple Silicon (M1/M2/M3/M4): `mix-agent-desktop_*_aarch64.dmg`
  - Intel Mac: `mix-agent-desktop_*_x64.dmg`
  - *Installation*: Open the `.dmg` and drag `MIX-agent Desktop` into your `Applications` folder.
  - *macOS Gatekeeper*: Pre-release builds are currently unsigned/unnotarized. If Gatekeeper blocks opening, right-click (or Control-click) the app icon and select "Open", or run `xattr -cr "/Applications/MIX-agent Desktop.app"` in Terminal.
- **Windows**: `mix-agent-desktop_*_x64-setup.exe` or `*.msi` installer
- **Linux**: `mix-agent-desktop_*_amd64.deb` or `*.AppImage` package

#### 2. Run / Build from Source

Requires Rust (1.82+), Node.js (22+), and pnpm.

```sh
# Install dependencies
pnpm install

# Launch in development mode
pnpm desktop:dev

# Build native macOS .dmg package locally
pnpm desktop:build:dmg
```

For full architecture and advanced options, refer to [`apps/desktop/README.md`](apps/desktop/README.md).

---

### Docker Edition (Home Server & Self-Hosted)

Self-host a private chat AI and autonomous agent workspace on home servers, HomeLabs, or NAS devices (ZimaOS / CasaOS). Run 24/7 without sending conversation data to third-party cloud services, accessible via web browser over your local LAN or VPN.

Requires Docker Desktop or Docker Engine with Compose v2. Clone and start with a single command without extra config files:

```sh
git clone https://github.com/taitai2661/MIX-agent.git
cd MIX-agent
docker compose up -d --build
```

Once up, open [http://localhost:8080](http://localhost:8080) (or `http://<your-server-ip>:8080`) in your browser to create your initial admin account.
When connecting to Ollama, LM Studio, or vLLM running on the host machine, use `host.docker.internal` instead of `localhost`.

#### Mounting a Project Workspace

To connect a repository to the agent workspace, mount it to `/project`. Instructions in `AGENTS.md` (or `CLAUDE.md`) will automatically load into each run:

```sh
MIX_PROJECT_DIR=/path/to/your/repo docker compose up -d
```

#### Updates & Backups

```sh
git pull --ff-only
docker compose up -d --build
```
State is safely persisted in Docker volumes (do not run `docker compose down -v` as it wipes volumes). Export an encrypted backup from the Admin UI prior to updates.

---

## Key Features (Core Engine)

Both editions share a unified agent engine with enterprise-grade capabilities:

### 1. Three Interaction Modes (Chat / Thinking / Agent)
- **Chat**: Fast conversational flow with tool execution on demand. Compatible even with non-reasoning models.
- **Thinking**: Harnesses reasoning models to deliberate before taking tool actions.
- **Agent**: Autonomous loop designed for deep, multi-step tasks. Includes periodic checkpoints, stagnation detection with self-correction, rigorous verification gates, and dynamic budget extensions.

### 2. 52 Provider Presets & Capability Probing
- Built-in presets for OpenAI, Anthropic, Gemini, OpenRouter, Groq, Mistral, DeepSeek, Cohere, Ollama, LM Studio, and more.
- **Capability Probe**: Automatically checks model tool-calling readiness using zero-side-effect probes before runs.

### 3. Tool Registry & Fine-Grained Permissions
- Rich tools for file editing, persistent terminal execution, web search, web browsing, memory, and knowledge retrieval.
- **Permission Gates**: Control execution via `Always Allow`, `Ask`, or `Deny`. Approved rules are scoped to specific paths or commands and persisted securely.

### 4. MCP (Model Context Protocol) Support
- Seamless support for stdio and Streamable HTTP / Remote MCP servers.
- Connect existing MCP servers to expand agent abilities instantly.

### 5. Skills & Knowledge Base
- Import and export standard `SKILL.md` (Agent Skills format).
- Chunk-based text knowledge search and project-specific instructions.

---

## Security Boundaries & Model

Security and isolation models differ by edition:

### Desktop Edition Security Model
- **OS Permissions**: Runs natively with the permissions of your logged-in desktop user.
- **Secrets Management**: API keys and tokens are securely stored in the native OS Keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service).
- **Interactive Gates**: High-impact actions (file writes, shell commands, browser automation) trigger interactive approval prompts (`Ask`) by default, scoped to specific paths or commands.

### Docker Edition Security Model
- **Container Isolation**: Execution runners run non-root with read-only root filesystems, dropped capabilities, and memory limits. Neither the host root nor the Docker socket are mounted.
- **Network Egress Proxy**: Runners connect exclusively through an egress proxy enforcing strict destination allowlists. Private IP ranges are blocked by default.
- **Network Binding**: Listens on `localhost:8080` by default. Use `MIX_BIND` and `PUBLIC_ORIGIN` behind HTTPS reverse proxies for LAN/VPN setups.

---

## Development

This repository is structured as a pnpm workspace monorepo:

```sh
# Install workspace dependencies
pnpm install

# --- Desktop Edition Development ---
pnpm desktop:dev        # Run native desktop app
pnpm desktop:ui:dev     # UI standalone dev server
pnpm desktop:ui:test    # UI unit tests
cargo test --workspace --manifest-path apps/desktop/Cargo.toml # Rust core tests

# --- Docker / Web Edition Development ---
pnpm web:dev            # Web UI dev server
pnpm web:test           # Web UI tests
docker compose --profile test run --rm test # Backend test suite
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

---

## Documentation Index

| Document | Content |
| :--- | :--- |
| [Desktop README](apps/desktop/README.md) | Desktop edition internals, tool specifications, and packaging |
| [Architecture Overview](docs/architecture.md) | High-level monorepo architecture and design philosophy |
| [Desktop Architecture](docs/desktop/architecture.md) | Desktop edition (Rust + Tauri 2) design and API mapping |
| [Operations Guide](docs/operations.md) | Docker edition administration, backups, and restores |
| [Verification Matrix](docs/verification.md) | Real-world provider and platform verification status |
| [ZimaOS Guide](docs/zimaos.md) | Deployment via ZimaOS / CasaOS Community Store |
| [Contributing Guide](CONTRIBUTING.md) | Contribution workflows and guidelines |

---

## License

Distributed under the [Apache License 2.0](LICENSE). See [NOTICE](NOTICE) for third-party acknowledgments.
