# MIX-agent v0.3.0

## 概要

v0.2.3以降の完成済みMIX-agentを、v0.3.0として正式に公開・配布します。Autoルーティング、Model Info連携、Memoryトレース、Run予算可視化、Skillsパッケージ、利用量集計、設定画面刷新、PWA対応、安定性・性能改善を含むリリースです。

## 更新内容

### Autoルーティング

- `providers/capability_probe.py` を追加し、Tool Calling / Vision / Reasoning を副作用のないダミー関数で自動確認します。
- 能力判定は `supported` / `unsupported` / `unknown` の3値で保存します。
- 未確定の判定は6時間、確定した判定は14日で自動失効します。手動Overrideは常に優先されます。
- `providers/model_quality.py` に、ローカルの控えめなロールメタデータと品質スコアを追加しました。プロプライエタリ判定や未公開メトリクスは使用しません。
- `providers/model_roles.py` に保守的なAutoルーティング用ロール定義を追加しました。
- `routing.py` を大幅に拡張し、役割ベースの自動モデル選択に対応しました。Reliability learningで集約した成功率をリトライとモデル選定へ反映します。
- `POST /models/{id}/verify-tools` を復活し、CLIトリガーで能力を再確認できます。

### Model Info（外部カタログ連携）

- `providers/model_info.py` を追加し、model-infoの静的JSONからContext Window、価格、能力、マルチモーダル情報を取得します。
- Provider Sync時に取得した情報をエビデンスとして保存します。
- 取得先は `MIX_MODEL_INFO_URL` で切り替えられます。GitHub PagesのHTTPS URL、または `v1/` を含むローカルmodel-infoチェックアウトに対応します。
- キャッシュTTLは10分です。取得に失敗しても処理をブロックせず、未知の情報として継続します。
- `metadata.py` をスリム化し、価格計算を新しい `pricing.py` へ分離しました。

### Memory

- `memory/service.py` を拡張し、関連Memoryを辿れる表示、ソース、鮮度、接続状態の一覧化に対応しました。
- `Memories.tsx` を全面改修し、Memory間の参照関係を辿れるUIを追加しました。
- Auto-traceの検証として `test_memory_auto_trace.py` を追加しました。

### Runの予算・状態可視化

- APIスキーマに `RunBudgetView` を追加しました。
- 新しい `RunBudget.tsx` で、残り時間と残りステップをサーバー取得値とクライアント側の時計減算で常時表示します。
- `RunActivity.tsx` を大幅に改修し、サイドパネルでストリームとTool実行履歴を同期表示します。

### Skillsパッケージ仕様

- `skills/package.py` を追加し、`SKILL.md` のパース、検証、レンダリング、`.zip` のエンコード・デコード、ホストディレクトリスキャンに対応しました。
- パッケージ処理を `service.py` から分離してテスト可能にしました。
- Unit / Integrationの `test_skill_package*.py` を追加しました。

### 利用量と料金

- `usage.py` を追加し、呼び出しごとのトークン使用量を本文なしで記録します。保存するのはカウントと価格だけで、保持期間は90日です。
- `pricing.py` でmodel-infoの価格表とフォールバック価格を利用します。
- 新しい `Usage.tsx` で、期間別・モデル別の集計と推移グラフを表示します。

### Chat操作

- ストリーム切断後の再接続時にメッセージとTool実行履歴を再同期するよう `Chat.tsx` を改善しました。
- 添付状態をリセットし、同じファイルを連続して再添付できるようにしました。
- `Chat.tsx`、`ArtifactCard.tsx`、`Conversations.tsx` の状態・表示整合性を改善しました。

### 設定画面と共通UI

- `Settings.tsx` / `Overview.tsx` を再構成し、ナビゲーションを刷新しました。
- 新しい `Network.tsx` で、Runner / Egress / MCP Proxyの通信許可ドメインを一元管理します。
- `Diagnostics.tsx`、`ToolSettings.tsx`、`Backups.tsx`、`Account.tsx`、`General.tsx`、`Statistics.tsx` を整理しました。
- Providersの接続テスト・モデル取得フローを改善しました。
- Models画面を刷新し、候補、確認済み、Overrideを視覚的に分離しました。
- `components/accent.tsx`、`select-menu.tsx`、`toggle.tsx` を追加・整理し、テーマ処理を改善しました。
- `style.css` を大幅に改修し、ダーク／ライト配色を再設計しました。
- PWA対応アイコン群、`manifest.webmanifest`、`OCTICONS-LICENSE` を追加しました。

### 安定性・性能・依存関係

- `reliability.py` を拡張し、直近の失敗・中断を診断画面へ提供します。
- `performance.py`、`runs/engine.py`、`runs/state.py` でステートマシンの取りこぼしを修正しました。
- `tools/registry.py` のツール権限判定を、fingerprint / call_scope / rule_scope / permissionの観点で堅牢化しました。
- OpenAIを2.54.0から3.17.0へ、MCPを1.29.1から2.2.0へ更新しました。

### バージョン情報の統一

以下のバージョン参照を `0.3.0` に更新しています。

- FastAPIアプリケーションのバージョン
- MCP ClientInfoのバージョン
- Pythonパッケージのバージョン（`pyproject.toml` / `uv.lock`）
- Webパッケージのバージョン
- OpenAPI仕様の `info.version`
- ZimaOS用Composeのアプリバージョン

### コンテナ配布

既存のGitHub Actionsによる公開方式を継続し、Gitタグ `v0.3.0` を起点に、次の8種類のAMD64向けコンテナイメージをGHCRへ公開しました。

- `ghcr.io/taitai2661/mix-agent-init:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-app:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-execution:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-browser-runner:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-browser-provisioner:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-mcp:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-mcp-runtime:v0.3.0`
- `ghcr.io/taitai2661/mix-agent-egress:v0.3.0`

### ZimaOS Community Store

ZimaOS向けの既存配布経路をそのまま利用し、v0.3.0のイメージ参照に更新して公開しました。

- ZimaOS v2向けストアメタデータ
- 旧CasaOS/ZimaOSクライアント向けv1 `store/main.zip`
- `Apps/MIX-agent/docker-compose.yml` 内の全イメージタグ
- `x-casaos.version` とリリースタグの一致
- GitHub Releaseに添付されたストアアーカイブ

ZimaOSのストアURLは従来どおりです。

- v2: `https://cdn.jsdelivr.net/gh/taitai2661/MIX-agent@gh-pages/store.json`
- v1互換: `https://cdn.jsdelivr.net/gh/taitai2661/MIX-agent@gh-pages/store/main.zip`

## 安全上の境界

- Tool Callingの自動確認は副作用のないダミー関数で行い、応答本文を保存しません。
- 利用量記録はトークン数と価格だけを保存し、本文、添付ファイル、Tool結果は保存しません。
- `MIX_MODEL_INFO_URL` が未設定、または外部カタログの取得に失敗しても処理をブロックせず、未知として継続します。
- 手動Overrideは自動判定より常に優先されます。
- Dockerのビルド方式、既存のGHCR公開方式、ZimaOSストアの公開方式は維持しています。

## 更新前の注意

更新前にMIX-agentの管理画面から暗号化バックアップを取得し、復元用パスフレーズを保管してください。ZimaOSストア版と手動Compose版ではデータ領域が異なる場合があり、既存データが自動的に引き継がれないことがあります。

MIX-agentは信頼できるLAN/VPN内での単一管理者利用を前提としています。初期管理者の作成前にインターネットへ公開しないでください。MCP管理用コンテナはDocker socketを使用します。

## リリース参照

- Git tag: `v0.3.0`
- GitHub Release: **MIX-agent v0.3.0**
- コンテナレジストリ: GitHub Container Registry（GHCR）
