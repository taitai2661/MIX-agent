# 公開手順

この文書はMIX-agentのバージョン公開作業の確認用です。`vX.Y.Z` は公開するタグに読み替えてください。

1. リポジトリの変更をレビューし、`Validate release candidate` が成功したことを確認する。
2. 実機または検証用Docker環境で新規導入を試し、管理者作成、Provider接続、Runner、MCPを確認する。
3. 既存データの暗号化バックアップを取得し、検証環境へ復元する。DB migration後の会話と設定を確認する。
4. 公開リポジトリのmainへ反映し、`vX.Y.Z` タグを作成する。
5. `Release MIX-agent` が8種類のAMD64イメージをGHCRへ配布し、その完了後にZimaOSストア公開が自動で続くことを確認する。
6. 初回公開時はGitHub Packagesで8種類のパッケージをPublicにし、匿名で各イメージをpull/manifest参照できることを確認する。
7. `gh-pages` に `store.json`、`index.json`、`store/main.zip` があり、対象バージョンが一致していることを確認する。
8. GitHub Releaseに `mix-agent-zimaos-store-vX.Y.Z.tar.gz` と `mix-agent-zimaos-legacy-vX.Y.Z.zip` が添付されていることを確認する。
9. `desktop-release` ワークフローが完了し、GitHub Release に Desktop 版の各種インストーラー（macOS 用 `.dmg` [aarch64 / x64]、Windows 用 `.msi` / `.exe`、Linux 用 `.deb` / `.AppImage`、および `SHA256SUMS`）が添付されていることを確認する。
10. ZimaOSでv2ストアURLから更新を確認し、必要なら旧クライアント向け `store/main.zip` でも追加・更新を確認する。
11. 配布された `.dmg` を macOS 実機でマウントし、Applications フォルダへの配置および起動ができることを確認する。
12. Desktop updaterについて、GitHub Actions Secretsの`TAURI_SIGNING_PRIVATE_KEY`が設定済みで、埋め込み公開鍵に対応することを安全な方法で確認する。値をログやReleaseへ出さない。
13. Releaseの`latest.json`に各生成対象のURLと署名があり、少なくとも1台の各OS実機で更新検出・検証・適用まで確認する。

公開前にチェックする点:

- `LICENSE`、`NOTICE` とアイコンのライセンス情報が残っている。
- API key、パスワード、個人データ、開発機の生成物が含まれていない。
- 設定ファイル・README・Releaseノートに非公開の作業用リポジトリへの参照が含まれていない。
- ZimaOSの既存手動導入からストア導入へのデータ引き継ぎは自動ではない。ガイドのバックアップ・復元説明を維持する。
- v2の`apps/MIX-agent/docker-compose.yml`とv1互換の`Apps/MIX-agent/docker-compose.yml`の`x-casaos.version`が、Releaseタグと一致している。
- Desktop updater署名の秘密鍵はActions Secretsだけで管理し、ソース・ビルドログ・Release assetsへ含めない。
