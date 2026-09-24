# ZimaOSコミュニティストア

## 配布の仕組み

公開リポジトリの `store-config.json` と `Apps/MIX-agent/docker-compose.yml` から、
新しいZimaOS向けのv2ストアと、旧クライアント互換のv1ストアを同時に生成します。

Releaseタグ `vX.Y.Z` を作成すると、GitHub Actionsは次の順番で自動処理します。

1. 8種類のAMD64コンテナイメージをGHCRへbuild/pushする。
2. 全イメージが匿名で参照できることを確認する。
3. v2の `store.json` / `index.json` / アプリメタデータを生成する。
4. v1互換の `store/main.zip` を生成して整合性を検証する。
5. `gh-pages` へ公開し、jsDelivrの主要ストアパスをpurgeする。
6. GitHub Releaseへv2+v1を含むストアbundleと、v1単体zipを添付する。

`Publish ZimaOS store` ワークフローは手動再公開にも使えます。タグを指定した手動実行では、
Compose内のバージョンとタグが一致し、8種類のGHCRイメージが揃っている場合だけ公開します。

## ZimaOSへの追加

新しいZimaOSではv2ストアURLを使います。

```text
https://cdn.jsdelivr.net/gh/taitai2661/MIX-agent@gh-pages/store.json
```

旧方式のストアURLが必要なクライアントでは、次を使用します。

```text
https://cdn.jsdelivr.net/gh/taitai2661/MIX-agent@gh-pages/store/main.zip
```

1. ZimaOSの「App Store → Community store」を開き、ストア追加（+）を選びます。
2. 利用しているZimaOSが対応するURLを入力し、App Storeを更新します。
3. MIX-agentを選択してインストールし、ZimaOSのLAN内アドレスの8080番ポートを開いて管理者を作成します。

GHCRの各コンテナパッケージがPrivateのままだとZimaOSから取得できません。
初回公開時は8種類すべてがPublicになっていることを確認してください。

## 動作上の条件

- AMD64のDocker環境を対象にしています。Web UIはホストの8080番ポートを使用します。
- 複数のコンテナ、PostgreSQL、Browser Runnerを起動します。容量とメモリに余裕が必要です。
- 信頼できるLAN/VPNで利用し、初回管理者作成前にインターネットへ公開しないでください。
- MCP管理用コンテナはDocker socketをマウントします。
- OAuthとWeb Pushを利用する場合は、アプリの環境変数 `PUBLIC_ORIGIN` を利用者がアクセスする実際のHTTPS URLに設定してください。
- ローカルのOllama/LM Studioへ接続する場合はProviderのプライベート接続を許可し、ホストの待受アドレスとファイアウォールも確認してください。

## 既存インストールの更新

既存の手動Composeインストールからストア版に切り替える場合は、すべてのRunを停止してから
アプリの暗号化バックアップを取得し、復元に必要なパスフレーズを保管してください。
初期版のバックアップには容量上限があり、パッケージキャッシュとBrowser Cookieは含まれません。
詳しくは[運用ガイド](operations.md)を確認してください。

ZimaOS側はストアごとにインストールを分離するため、同名のComposeでも既存のDockerボリュームを
自動で引き継ぐとは限りません。既存スタックを止めてストア版を起動し、一時的な管理者を作成してから復元します。
復元後はバックアップに含まれる管理者で再ログインし、会話・Provider設定・添付・Tool実行環境を確認してください。
必要なMCPパッケージは再導入します。ボリューム削除は復元確認まで行わないでください。

## メンテナー向け

生成済みZimaOS Composeが現在のバージョンと一致するかは次で確認できます。

```sh
python -m pip install PyYAML==6.0.2
VERSION="$(awk '/^  version:/ {gsub(/["\047]/, "", $2); print $2; exit}' Apps/MIX-agent/docker-compose.yml)"
python scripts/render_zimaos_compose.py "v$VERSION"
docker compose -f Apps/MIX-agent/docker-compose.yml config --quiet
```

通常の `compose.yaml` にサービスを追加・変更した場合は、生成ファイルも更新してください。
公開前には、DB migration、アプリテスト、全release image build、v1 archive build、フロントエンドbuildを
`Validate release candidate` で確認し、実際のZimaOS上でも新規導入・復元・起動を確認してください。
