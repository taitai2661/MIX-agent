# 公開手順

この文書はMIX-agentのバージョン公開作業の確認用です。`vX.Y.Z` は公開するタグに読み替えてください。

1. リポジトリの変更をレビューし、`Validate release candidate` が成功したことを確認する。
2. 実機または検証用Docker環境で新規導入を試し、管理者作成、Provider接続、Runner、MCPを確認する。
3. 既存データの暗号化バックアップを取得し、検証環境へ復元する。DB migration後の会話と設定を確認する。
4. 公開リポジトリのmainへ反映し、必須CIが成功したことを確認する。
5. `desktop-release` をmain refで手動実行し、`tag=main`、`dry_run=true` を指定する。Apple Silicon／Intel macOSの署名build、`latest.json`検証、埋め込み公開鍵による署名検証が成功し、release jobがskipされたことを確認する。秘密鍵の値やActions Secretsの内容は読み出さない。
6. `vX.Y.Z` タグを作成し、`Release MIX-agent` が8種類のAMD64イメージをGHCRへ配布した後、ZimaOSストア公開が自動で続くことを確認する。
7. 初回公開時はGitHub Packagesで8種類のパッケージをPublicにし、匿名で各イメージをpull/manifest参照できることを確認する。
8. `gh-pages` に `store.json`、`index.json`、`store/main.zip` があり、対象バージョンが一致していることを確認する。
9. GitHub Releaseに `mix-agent-zimaos-store-vX.Y.Z.tar.gz` と `mix-agent-zimaos-legacy-vX.Y.Z.zip` が添付されていることを確認する。
10. `desktop-release` ワークフローが完了し、GitHub Release に Desktop 版のmacOS用`.dmg`（aarch64 / x64）と`SHA256SUMS`が添付されていることを確認する。WindowsおよびLinux向けDesktopインストーラーはv0.5.0では公開しない。
11. ZimaOSでv2ストアURLから更新を確認し、必要なら旧クライアント向け `store/main.zip` でも追加・更新を確認する。
12. 配布された `.dmg` を macOS 実機でマウントし、Applications フォルダへの配置および起動ができることを確認する。
13. Releaseの`latest.json`にApple Silicon／Intel macOSのURLと署名があることを確認し、macOS実機で更新検出・検証・適用まで確認する。WindowsおよびLinux向けDesktop更新は今回の対象外。秘密鍵はActions Secretsのみで管理し、値をログやReleaseへ出さない。

公開前にチェックする点:

- `LICENSE`、`NOTICE` とアイコンのライセンス情報が残っている。
- API key、パスワード、個人データ、開発機の生成物が含まれていない。
- 設定ファイル・README・Releaseノートに非公開の作業用リポジトリへの参照が含まれていない。
- ZimaOSの既存手動導入からストア導入へのデータ引き継ぎは自動ではない。ガイドのバックアップ・復元説明を維持する。
- v2の`apps/MIX-agent/docker-compose.yml`とv1互換の`Apps/MIX-agent/docker-compose.yml`の`x-casaos.version`が、Releaseタグと一致している。
- Desktop updater署名の秘密鍵はActions Secretsだけで管理し、ソース・ビルドログ・Release assetsへ含めない。
