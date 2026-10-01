"""Build the ZimaOS store Compose from the regular deployment Compose.

Run with: python scripts/render_zimaos_compose.py v0.3.5
PyYAML is required for this maintainer-only script.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "Apps/MIX-agent/docker-compose.yml"
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
        raise ValueError("Expected a release tag such as v0.3.5")
    source = yaml.safe_load((ROOT / "compose.yaml").read_text())
    source.pop("x-restricted", None)
    source.pop("secrets", None)
    for name in list(source["services"]):
        if name == "db":
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
        "update_at": "2026-09-24",
        "release_notes": {
            "en_US": "Fix LAN HTTP setup compatibility and improve chat and memory behavior.",
            "ja_JP": "LAN内HTTPでの初期設定互換性を修正し、ChatとMemoryの挙動を改善。",
        },
    }
    return "# Generated by scripts/render_zimaos_compose.py; edit the generator.\n" + yaml.safe_dump(
        source, allow_unicode=True, sort_keys=False, width=110
    )


if __name__ == "__main__":
    OUT.write_text(render(sys.argv[1] if len(sys.argv) == 2 else "v0.3.5"))
