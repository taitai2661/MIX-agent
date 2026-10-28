"""Build the ZimaOS store Compose from the regular deployment Compose.

Run with: python scripts/render_zimaos_compose.py v0.5.4
PyYAML is required for this maintainer-only script.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
# The v2 store source uses `apps/`. Both upstream store builders need an
# `Apps/` tree, but committing both spellings breaks case-insensitive
# filesystems; the release workflows stage it with scripts/stage-store-source.sh.
OUTS = (ROOT / "apps/MIX-agent/docker-compose.yml",)
PACKAGE = "ghcr.io/taitai2661/mix-agent"
TARGETS = {
    "init": "init",
    "app": "app",
    "execution-runner": "execution",
    "browser-runner": "browser-runner",
    "browser-provisioner": "browser-provisioner",
    "mcp-runner": "mcp",
    "mcp-manager": "mcp-runtime",
    "egress-proxy": "egress",
}
# Host mounts a store install keeps: the MCP manager needs the Docker socket to
# attach per-server containers to their own networks.
ALLOWED_BIND_MOUNTS = ("/var/run/docker.sock",)

# Store metadata per release tag. The ZimaOS store shows these one-liners
# next to the app, so they cannot be derived from the code. They live here,
# keyed by tag, and the store Compose is always *generated* from them —
# editing the generated file by hand is what let v0.2.3-era prose and a
# stale image list survive into later releases.
#
# A tag without an entry is an error on purpose: a wrong one-liner published
# to the store is worse than a failed generation.
RELEASE_META = {
    "v0.5.4": {
        "update_at": "2026-10-28",
        "notes": {
            "en_US": (
                "Desktop (beta) installs Chromium on demand, manages local LLM "
                "runtimes, and adds a live browser view. Desktop/Web browser "
                "actions are expanded."
            ),
            "ja_JP": (
                "Desktop版（ベータ）にChromiumの初回インストール、"
                "ローカルLLMランタイム管理、ブラウザのライブ表示を追加。"
                "Desktop/Web版のブラウザ操作を拡張。"
            ),
        },
    },
    "v0.5.2": {
        "update_at": "2026-10-27",
        "notes": {
            "en_US": (
                "Desktop (beta) gains signed in-app updates and Linux x64 "
                "AppImage/deb builds. Docker/Web functionality is unchanged."
            ),
            "ja_JP": (
                "Desktop版（ベータ）に署名付きアプリ内アップデートと"
                "Linux x64（AppImage/deb）配布を追加。Docker/Web版の機能変更はありません。"
            ),
        },
    },
    "v0.5.1": {
        "update_at": "2026-10-27",
        "notes": {
            "en_US": (
                "Fixes the macOS DMG so the app launches on Apple Silicon, adds "
                "Windows x64 installers, and polishes the desktop terminal and "
                "platform-specific shortcuts."
            ),
            "ja_JP": (
                "Apple SiliconのmacOSで起動できるようDMGを修正し、"
                "Windows x64インストーラーを追加。Desktopのターミナルと"
                "プラットフォーム別ショートカットを修正。"
            ),
        },
    },
    "v0.5.0": {
        "update_at": "2026-10-25",
        "notes": {
            "en_US": (
                "Adds the Desktop edition, moves model metadata to models.dev, "
                "and updates Memory, long-running agents, and release packaging."
            ),
            "ja_JP": (
                "Desktop版を追加し、モデル情報をmodels.devへ移行。"
                "Memory・長時間Agent実行基盤・配布ワークフローを更新。"
            ),
        },
    },
}


def _is_bind_mount(volume: str) -> bool:
    """True for a host bind mount, which a store install must not inherit.

    A store app is unpacked under the store's own sysroot and the dashboard
    supplies its data directories as named volumes. A host path carried over
    from ``compose.yaml`` would instead resolve relative to that sysroot and
    mount the install directory itself into the container — read-write for
    ``./skills``, which would write imported skills into the store.

    ``ALLOWED_BIND_MOUNTS`` is the exception: the MCP manager joins
    per-server Docker networks through the host socket.
    """
    source = volume.split(":", 1)[0].strip()
    if source in ALLOWED_BIND_MOUNTS:
        return False
    return source.startswith((".", "/", "~", "$")) or "/" in source or ":" in source


def render(tag: str) -> str:
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag):
        raise ValueError("Expected a release tag such as v0.2.3")
    if tag not in RELEASE_META:
        raise ValueError(
            f"No store metadata for {tag}. Add a RELEASE_META entry "
            f"(update_at + en_US/ja_JP notes) before generating the store Compose."
        )
    meta = RELEASE_META[tag]
    source = yaml.safe_load((ROOT / "compose.yaml").read_text())
    source.pop("x-restricted", None)
    source.pop("secrets", None)
    for name in list(source["services"]):
        # `db` and the Docker socket proxy are infrastructure services, not
        # release images: keep them verbatim (the proxy image is pinned by
        # digest in compose.yaml and must not be rewritten to a GHCR tag).
        if name in ("db", "docker-socket-proxy"):
            continue
        if name not in TARGETS:
            del source["services"][name]
            continue
        service = source["services"][name]
        service.pop("build", None)
        service["image"] = f"{PACKAGE}-{TARGETS[name]}:{tag}"
        # Named volumes only; see _is_bind_mount.
        service["volumes"] = [
            volume
            for volume in service.get("volumes", [])
            if not _is_bind_mount(volume)
        ]
    # The ZimaOS dashboard is reached by private LAN IP. OAuth and push users
    # must set PUBLIC_ORIGIN to their actual HTTPS origin after installation.
    source["services"]["app"]["ports"] = ["8080:8080"]
    source["services"]["mcp-manager"]["environment"]["MCP_RUNTIME_IMAGE"] = (
        f"{PACKAGE}-mcp-runtime:{tag}"
    )
    # MCP manager connects this exact container to per-server Docker networks.
    # A store may rewrite the Compose project name, so keep this name stable.
    source["services"]["egress-proxy"]["container_name"] = "mix-agent-egress-proxy-1"
    source["x-casaos"] = {
        "id": "io.github.taitai2661.mix-agent",
        "main": "app",
        "index": "/",
        "port_map": "8080",
        "scheme": "http",
        "title": {"en_US": "MIX-agent", "ja_JP": "MIX-agent"},
        "tagline": {
            "en_US": "Your AI workspace on a trusted home network.",
            "ja_JP": "家庭内ネットワークで使うAIワークスペース。",
        },
        "description": {
            "en_US": (
                "Self-hosted chat, agents, tools and MCP. For one administrator "
                "on a trusted LAN or VPN; not suitable for internet exposure."
            ),
            "ja_JP": (
                "チャット・Agent・Tool・MCPをまとめたセルフホスト環境。"
                "信頼できるLAN/VPNの単一管理者向け。インターネット公開は非推奨。"
            ),
        },
        "tips": {
            "before_install": {
                "en_US": (
                    "Uses port 8080 and multiple containers including PostgreSQL, "
                    "browser and MCP runners. Requires significant memory and disk. "
                    "Install only on a trusted LAN/VPN. MCP management mounts the "
                    "Docker socket. Back up existing MIX-agent data before an upgrade. "
                    "Set PUBLIC_ORIGIN to your real HTTPS URL for OAuth and web push."
                ),
                "ja_JP": (
                    "8080番ポートとPostgreSQL・Browser・MCPなど複数コンテナを使用します。"
                    "十分なメモリと空き容量が必要です。信頼できるLAN/VPN内でのみ使用してください。"
                    "MCP管理用コンテナはDocker socketをマウントします。更新前にデータをバックアップし、"
                    "OAuth・Web Push利用時はPUBLIC_ORIGINに実際のHTTPS URLを設定してください。"
                ),
            }
        },
        "author": "MIX-agent contributors",
        "developer": "MIX-agent contributors",
        "category": "AI",
        "architectures": ["amd64"],
        "version": tag.removeprefix("v"),
        "update_at": meta["update_at"],
        "release_notes": meta["notes"],
    }
    return "# Generated by scripts/render_zimaos_compose.py; edit the generator.\n" + yaml.safe_dump(
        source, allow_unicode=True, sort_keys=False, width=110
    )


if __name__ == "__main__":
    rendered = render(sys.argv[1] if len(sys.argv) == 2 else "v0.5.2")
    for output in OUTS:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(rendered)
