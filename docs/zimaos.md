# ZimaOSコミュニティストア

## 配布の仕組み

公開リポジトリの `store-config.json` と `Apps/MIX-agent/docker-compose.yml` から
ZimaOS v2用の `store.json` とアプリ情報を生成します。Releaseタグ `v0.2.0`
を作成すると、GitHub ActionsがAMD64のコンテナイメージをGHCRへ配布します。
初回の配布では8つのGHCRパッケージをPublicに変更し、公開イメージを確認した後で
`Publish store and GitHub Release` ワークフローにタグを入力します。
そこでストアを `gh-pages` ブランチへ配置し、GitHub Releaseを作成します。

公開後の追加手順:

1. ZimaOSの「App Store → Community store」を開き、ストア追加（+）を選びます。
2. 次のURLを入力して追加し、App Storeを更新します。
3. MIX-agentを選択してインストールし、ZimaOSのLAN内アドレスの8080番ポートを開いて管理者を作成します。

```text
https://cdn.jsdelivr.net/gh/taitai2661/MIX-agent@gh-pages/store.json
```

配布前のURLは利用できません。GHCRの各コンテナパッケージは、初回の
イメージ配布後にPublicになっていることを確認してください。Privateのままでは
ZimaOSから取得できません。

## 動作上の条件

- AMD64のDocker環境を対象にしています。Web UIはホストの8080番ポートを使用します。
- 複数のコンテナ、PostgreSQL、Browser Runnerを起動します。容量とメモリに余裕が必要です。
- 信頼できるLAN/VPNで利用し、初回管理者作成前にインターネットへ公開しないでください。
- MCP管理用コンテナはDocker socketをマウントします。
- OAuthとWeb Pushを利用する場合は、アプリの環境変数 `PUBLIC_ORIGIN` を
  利用者がアクセスする実際のHTTPS URLに設定してください。
- ローカルのOllama/LM Studioへ接続する場合はProviderのプライベート接続を許可し、
  ホストの待受アドレスとファイアウォールも確認してください。

## 既存インストールの更新

既存の手動Composeインストールからストア版に切り替える場合は、すべてのRunを
停止してからアプリの暗号化バックアップを取得し、復元に必要なパスフレーズを
保管してください。初期版のバックアップには容量上限があり、パッケージキャッシュと
Browser Cookieは含まれません。詳しくは[運用ガイド](operations.md)を確認してください。
ZimaOS側はストアごとにインストールを分離するため、同名のComposeでも
既存のDockerボリュームを自動で引き継ぐとは限りません。
既存スタックを止めてストア版を起動し、一時的な管理者を作成してから復元します。
復元後はバックアップに含まれる管理者で再ログインし、会話・Provider設定・添付・
Tool実行環境を確認してください。必要なMCPパッケージは再導入します。
ボリューム削除は復元確認まで行わないでください。

## メンテナー向け

```sh
python -m pip install PyYAML==6.0.2
python scripts/render_zimaos_compose.py v0.2.0
docker compose -f Apps/MIX-agent/docker-compose.yml config --quiet
```

通常の `compose.yaml` にサービスを追加・変更した場合は、生成ファイルも更新してください。
公開前には、全イメージのビルド、DB migrationとアプリのテスト、
実際のZimaOS上での新規導入・復元・起動を確認してください。
