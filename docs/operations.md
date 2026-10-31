# 運用

## 起動・停止・ログ

```sh
docker compose up -d --build
docker compose ps
docker compose logs --tail=100 app
docker compose stop
```

`docker compose down -v` は永続データを削除するため通常の停止には使わないでください。

Docker初回起動時のinitがDBパスワード、内部トークン、暗号鍵を生成します。既存の値は上書きしません。Runnerはread-only filesystem、非root、Capability削除、メモリ/CPU/PID上限で起動します。

### 構造化ログ

既定は従来どおり人が読むテキスト形式です。ログ収集基盤へ渡す場合は `MIX_LOG_FORMAT=json` を設定すると、1行1オブジェクトのJSON（`ts` / `level` / `logger` / `message` と、呼び出し側が付けた `request_id` などの追加フィールド）になります。

```sh
MIX_LOG_FORMAT=json docker compose up -d
```

`request_id` はレスポンスの `X-Request-ID` と一致するため、ユーザーから報告されたIDで該当リクエストを検索できます。

`db` と `x-restricted` を使う各コンテナのログは、Docker の `json-file` ドライバで 10 MiB × 3 ファイルに制限しています。長期保存が必要な場合はこの上限を上げず、外部のログ収集基盤へ転送してください。

## ヘルスチェックとメトリクス

Compose の `app` には `/live` を使う healthcheck があります。`/live` はDBに触れないため、DBが一時的に落ちてもアプリを再起動しません。DB接続まで含めた準備確認は `/health` を使ってください。

`/metrics` は Prometheus 形式のプロセス内カウンタを返します（HTTPメソッド・ルートテンプレート・ステータス別の件数、レイテンシの summary、稼働時間）。ラベルは `GET /api/v1/runs` のようにルート単位へ丸めるため、Run IDや任意の404パスがそのままラベルになることはありません。`METRICS_TOKEN` が未設定の場合、`/metrics` は loopback とプライベートネットワークからのアクセスにのみ応答し、それ以外は404を返します。外部のPrometheusから直接取得する場合は `METRICS_TOKEN` を設定して `Authorization: Bearer <token>` を要求するか、LAN内に閉じた scrape 経路を用意してください。`MIX_METRICS_PUBLIC=1` を設定すると従来どおり無認証で全クライアントへ公開されます（非推奨）。複数workerで動かす場合、カウンタはプロセスごとに分かれる点に注意してください。

Reverse Proxy 配下では、アプリから見た TCP ピアは Proxy のアドレスです。`MIX_TRUSTED_PROXIES` にその Proxy の IP/CIDR（例: `172.16.0.0/12`）を設定すると、その Proxy から届いた `X-Forwarded-For` だけを使って `/metrics` の公開判定とログイン試行制限のクライアントIPを解決します。未設定時の信頼対象は loopback（`127.0.0.1,::1`）のみで、それ以外のクライアントが送った `X-Forwarded-For` は無視されます。Proxy のアドレスを誤って広く指定するとIP偽装を許すため、必要なサブネットだけを指定してください。

`mcp-manager` はホストのDocker Socketを直接マウントしません。`docker-socket-proxy` 経由でコンテナ・ボリューム・ネットワーク・イメージのAPIだけを許可し、build/swarm/secrets等の管理APIを減らしています。ただしMCPの導入にはコンテナ作成APIが必要で、managerが侵害された場合の影響は依然として大きいため、信頼できるネットワーク（`mcp` internal network）に限定しています。

## Browser Runtimeの更新

Browser Toolは専用RunnerのPlaywright Chromiumを使います。依存関係の更新候補はDependabotが毎週作成します。更新を取り込んだら、通常どおり `docker compose up -d --build` でexecution-runnerを再ビルドしてください。これによりPlaywrightと対応するChromium shellも一緒に更新されます。

実行中のRunnerがインターネットから自己更新したり、ホストのDocker Socketを操作したりすることはありません。更新はレビュー可能な依存関係変更とDockerイメージ再ビルドに限定します。

アプリDBはCompose内のPostgreSQL専用です。DBポートはホストへ公開せず、アプリはDBのhealthcheckとAlembic migration完了後に起動します。テストは `docker compose --profile test run --rm test`、画面確認は `docker compose --profile preview up --build preview` を使い、それぞれ使い捨てPostgreSQLを使用します。

## 通信許可（Egress）

`policy.json` は許可ドメインの一覧だけでなく `mode` を持ちます。`public_web` は公開ホストへの接続を許可し、`restricted` は `allowed_domains` との完全一致のみ、`none` は全拒否です。初期状態は `public_web`（`allowed_domains: []`）で、設定画面で許可ドメインを1件以上登録すると `restricted` に切り替わります。`allowed_domains` が空の `restricted` は「全拒否」であり、意図的に許可を外した状態と壊れたポリシーを取り違えません。古い `mode` の無いファイルは従来どおり（空 = `public_web`、非空 = `restricted`）解釈されますが、起動ログに推測した mode が警告として出ます。

Private IP・link-local・cloud metadata 宛の接続は mode に関係なく拒否されます。

## LAN/VPNとTLS

Dockerの公開ポートへLANのプライベートIPアドレスで直接アクセスする場合は、初期設定を含め、そのIPアドレス・ポートと一致するOriginが自動的に許可されます。localhostからのアクセスも従来どおり利用できます。HTTPS対応Reverse Proxy、独自ドメイン、またはインターネット公開では、ホスト側で `MIX_BIND` と正確な `PUBLIC_ORIGIN` を設定してappを再作成してください。これらはDocker起動境界の設定なのでWeb UIでは変更しません。LAN利用にもHTTPS対応Reverse ProxyまたはVPNを推奨します。不特定ユーザーへ公開する構成ではありません。

## MCP

Filesystemテンプレートを導入する場合は「一般・通信」で `registry.npmjs.org` を許可してから、MCP画面の「パッケージを導入」を実行します。依存取得先が別ドメインなら追加が必要です。初期版はnpmのinstall scriptsを実行しません。

Filesystemの接続設定は `/packages/node_modules/.bin/mcp-server-filesystem`、引数は `["/shared"]`。これはTerminalの `/workspace` と別のMCP専用領域です。

汎用stdioはインストール済み実行ファイルを指定します。npx/uvxによるTool実行時の自動ダウンロードは禁止します。初期版の自動パッケージ導入はFilesystemのみです。他の実行環境が必要な場合はRunnerイメージを拡張してください。Remote HTTPはHTTPS、Bearer/カスタムHeader、OAuth認証フローに対応します。OAuthは接続先Serverとの実接続確認が必要です。private IP接続は未対応です。

接続の編集後は既存Toolを無効化し、「接続・Tool取得」をやり直します。Secret欄は空欄で保持、`{}`で削除します。MCP Serverの標準エラーはSecret漏えいを避けてアプリログに流しません。

## バックアップ・復元

全RunとバックグラウンドProcessを停止し、設定画面で12文字以上のパスフレーズを入力してダウンロードします。バックアップはAES-GCM + scrypt。DB、暗号鍵、添付、workspace、MCP共有領域を含みます。

初期版の上限は暗号化前256MB、Runner領域ごと64MB・1万ファイル。symlink・特殊ファイルは非対応です。パッケージキャッシュ・Browser Cookieはバックアップしません。新しい環境への復元後、必要なMCPパッケージを再導入してください。

復元には管理者ログインが必要です。空の環境では一時的な管理者を作成してから復元します。復元後はバックアップに含まれる管理者で再ログインします。

復元前の暗号化退避はdata volumeの `restore-rollback.mix`、古い添付は `rollback-*` に残ります。Runnerも `.mix-rollback-*` に以前のファイルを保持します。自動削除はしません。これらの退避はディスクを消費するので、検証済みバックアップを別に確保してから管理者が整理してください。

復元途中に停止した場合、起動時に退避アーカイブから回復します。回復用のパスフレーズは鍵用volumeの `restore-passphrase` に master key で暗号化して一時保存し、完了時に削除します。Runnerに接続できず回復できない場合は起動を失敗させます。鍵volumeも含めて保全してください。DBだけのバックアップではSecretを復号できません。

## 開発用画面確認

画面検証はCompose profile `preview` の使い捨てPostgreSQL環境で行います。テストProviderは `tests/fixtures/fake_provider.py` にあり、実AIではない旨を返します。製品のProviderフォールバックには使用しません。
