# MIX-agent

個人・LAN/VPN内向けのセルフホスト AI ワークスペース。

このリポジトリは 2 つのエディションを **兄弟関係** で配布しています。Desktop 版は Docker 版の中核機能（Provider Adapter / Run Engine / Tool Registry / Run イベントの `kind` 契約）を踏襲しつつ、Tauri 2 + Rust で再実装されています。`docker/` と `desktop/` の間にコード共有はなく、仕様書（`docs/`）と契約のみが両者を結びます。

## エディション

| エディション | ディレクトリ | 対象 | 概要 |
| --- | --- | --- | --- |
| Docker 版 | [`docker/MIX-agent/`](docker/MIX-agent/) | LAN/VPN 内のセルフホスト | React + FastAPI + PostgreSQL。複数コンテナで分離。 |
| Desktop 版 | [`desktop/`](desktop/) | 個人 PC | Tauri 2 + Rust + React。OS Keychain 連携、Chromium 内蔵。 |

## クイックリンク

- Docker 版: [`docker/MIX-agent/README.md`](docker/MIX-agent/README.md)
- Desktop 版: [`desktop/README.md`](desktop/README.md)
- 設計資料: [`docs/desktop/`](docs/desktop/)（Desktop 設計）、`docker/MIX-agent/docs/`（Docker 設計）
- ライセンス: [`LICENSE`](LICENSE) / [`NOTICE`](NOTICE)

## 開発方針

- **依存方向**: `desktop/` → `docker/` への依存は禁止。逆方向も禁止。
- **データ共有**: 将来 `desktop/` から `docker/` 形式へのエクスポート／インポートを追加する可能性はありますが、両者の内部スキーマは独立して進化します。
- **ライセンス**: 両方とも Apache License 2.0 ([`LICENSE`](LICENSE))。コントリビューション前に [`docker/MIX-agent/CONTRIBUTING.md`](docker/MIX-agent/CONTRIBUTING.md) を確認してください。

## ステータス

| エディション | 状態 |
| --- | --- |
| Docker 版 | v0.2.3 をリリース済み（既存機能）。変更内容は [`docker/MIX-agent/docs/release-notes-v0.2.3.md`](docker/MIX-agent/docs/release-notes-v0.2.3.md) |
| Desktop 版 | Phase 0〜6 完了。Phase 7（macOS 完成 + Windows α + Linux α）は未着手。フェーズの内訳は [`desktop/README.md`](desktop/README.md) の「開発フェーズ」を参照 |

## アイコンの再生成

アプリ／favicon のアイコンは `scripts/generate-icons.py` が共通の幾何学マークから
ラスタ画像を生成します（Octicons 等の外部素材は使いません）。

```sh
python3 -m venv .venv
.venv/bin/pip install -r scripts/requirements.txt
.venv/bin/python scripts/generate-icons.py
```

生成先: `docker/MIX-agent/apps/web/public/`（favicon / PWA 用）、
`docker/MIX-agent/apps/MIX-agent/icon.svg`、
`desktop/mix-agent-desktop/app/icons/`、`desktop/mix-agent-desktop/ui/public/`。

ベクタの正本は `docker/MIX-agent/apps/web/public/icon.svg` で、スクリプトはそこから
他の 3 箇所へ SVG をコピーします。PNG / ICO はスクリプト内で直接生成します。
依存は開発用の Pillow のみで、配布物には含まれません。
