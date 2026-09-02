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

## Browser Runtimeの更新

Browser Toolは専用RunnerのPlaywright Chromiumを使います。依存関係の更新候補はDependabotが毎週作成します。更新を取り込んだら、通常どおり `docker compose up -d --build` でexecution-runnerを再ビルドしてください。これによりPlaywrightと対応するChromium shellも一緒に更新されます。

実行中のRunnerがインターネットから自己更新したり、ホストのDocker Socketを操作したりすることはありません。更新はレビュー可能な依存関係変更とDockerイメージ再ビルドに限定します。

アプリDBはCompose内のPostgreSQL専用です。DBポートはホストへ公開せず、アプリはDBのhealthcheckとAlembic migration完了後に起動します。テストは `docker compose --profile test run --rm test`、画面確認は `docker compose --profile preview up --build preview` を使い、それぞれ使い捨てPostgreSQLを使用します。

## LAN/VPNとTLS

Dockerの公開ポートへLANのプライベートIPアドレスで直接アクセスする場合は、初期設定を含め、そのIPアドレス・ポートと一致するOriginが自動的に許可されます。localhostからのアクセスも従来どおり利用できます。HTTPS対応Reverse Proxy、独自ドメイン、またはインターネット公開では、ホスト側で `MIX_BIND` と正確な `PUBLIC_ORIGIN` を設定してappを再作成してください。これらはDocker起動境界の設定なのでWeb UIでは変更しません。LAN利用にもHTTPS対応Reverse ProxyまたはVPNを推奨します。不特定ユーザーへ公開する構成ではありません。

## MCP

Filesystemテンプレートを導入する場合は「一般・通信」で `registry.npmjs.org` を許可してから、MCP画面の「パッケージを導入」を実行します。依存取得先が別ドメインなら追加が必要です。初期版はnpmのinstall scriptsを実行しません。

Filesystemの接続設定は `/packages/node_modules/.bin/mcp-server-filesystem`、引数は `["/shared"]`。これはTerminalの `/workspace` と別のMCP専用領域です。

汎用stdioはインストール済み実行ファイルを指定します。npx/uvxによるTool実行時の自動ダウンロードは禁止します。初期版の自動パッケージ導入はFilesystemのみです。他の実行環境が必要な場合はRunnerイメージを拡張してください。Remote HTTPはHTTPS、Bearer/カスタムHeaderに対応し、OAuthやprivate IP接続は未対応です。

接続の編集後は既存Toolを無効化し、「接続・Tool取得」をやり直します。Secret欄は空欄で保持、`{}`で削除します。MCP Serverの標準エラーはSecret漏えいを避けてアプリログに流しません。

## バックアップ・復元

全RunとバックグラウンドProcessを停止し、設定画面で12文字以上のパスフレーズを入力してダウンロードします。バックアップはAES-GCM + scrypt。DB、暗号鍵、添付、workspace、MCP共有領域を含みます。

初期版の上限は暗号化前256MB、Runner領域ごと64MB・1万ファイル。symlink・特殊ファイルは非対応です。パッケージキャッシュ・Browser Cookieはバックアップしません。新しい環境への復元後、必要なMCPパッケージを再導入してください。

復元には管理者ログインが必要です。空の環境では一時的な管理者を作成してから復元します。復元後はバックアップに含まれる管理者で再ログインします。

復元前の暗号化退避はdata volumeの `restore-rollback.mix`、古い添付は `rollback-*` に残ります。Runnerも `.mix-rollback-*` に以前のファイルを保持します。自動削除はしません。これらの退避はディスクを消費するので、検証済みバックアップを別に確保してから管理者が整理してください。

復元途中に停止した場合、起動時に退避アーカイブから回復します。回復中のパスフレーズは鍵用volumeに一時保存し、完了時に削除します。Runnerに接続できず回復できない場合は起動を失敗させます。鍵volumeも含めて保全してください。DBだけのバックアップではSecretを復号できません。

## 開発用画面確認

画面検証はCompose profile `preview` の使い捨てPostgreSQL環境で行います。テストProviderは `tests/fixtures/fake_provider.py` にあり、実AIではない旨を返します。製品のProviderフォールバックには使用しません。
