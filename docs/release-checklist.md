# 公開手順（リリースチェックリスト）

この文書はMIX-agentのバージョン公開作業の確認用です。`vX.Y.Z` は公開するタグに読み替えてください。

1. リポジトリの変更をレビューし、`Validate release candidate` が成功したことを確認する。
2. 実機または検証用Docker環境で新規導入を試し、管理者作成、Provider接続、Runner、MCPを確認する。
3. 既存データの暗号化バックアップを取得し、検証環境へ復元する。DB migration後の会話と設定を確認する。
4. 公開リポジトリのmainへ反映し、必須CIが成功したことを確認する。
5. `desktop-release` をmain refで手動実行し、`tag=main`、`dry_run=true` を指定する。Apple Silicon／Intel macOSのDMG、Windows x64のNSIS／MSI、Linux x64のAppImage／debがbuildされ、6ファイル分の`SHA256SUMS`と署名済みペイロードから生成した`latest.json`が作成されることを確認する。`TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` がRepository Secretsに登録済みであること。macOS `.app` は既定ではDeveloper ID署名・公証なしだが、必ずad-hoc署名され、`Verify macOS signature and bundle architecture` が成功すること（`codesign --verify --deep --strict` とアプリ本体のCPUアーキテクチャ一致）。`APPLE_*` Secretsを登録した場合は、同ジョブがDeveloper ID署名・公証・stapleまで検証する。Chromiumは同梱されず、初回利用時に`browser/src/chromium-manifest.json`のSHA-256で検証してダウンロードされる。
6. `vX.Y.Z` タグを作成し、`Release MIX-agent` が8種類のAMD64イメージをGHCRへ配布した後、ZimaOSストア公開が自動で続くことを確認する。
7. 初回公開時はGitHub Packagesで8種類のパッケージをPublicにし、匿名で各イメージをpull/manifest参照できることを確認する。
8. `gh-pages` に `store.json`、`index.json`、`store/main.zip` があり、対象バージョンが一致していることを確認する。
9. GitHub Releaseに `mix-agent-zimaos-store-vX.Y.Z.tar.gz` と `mix-agent-zimaos-legacy-vX.Y.Z.zip` が添付されていることを確認する。
10. `desktop-release` ワークフローが完了し、GitHub Release にmacOS用`.dmg`（Apple Silicon / Intel）2件、Windows用`-Windows-x64-Setup.exe`と`-Windows-x64.msi`、Linux用`.AppImage`と`.deb`、アップデータ用の`.app.tar.gz`／`-setup.exe`／`.AppImage` と各`.sig`、`latest.json`、`SHA256SUMS`が添付されていることを確認する。macOSアプリ本体はApple Developer ID署名・公証なし（ad-hoc署名のみ）、Windowsインストーラーも未署名。macOS Gatekeeperの初回確認（「このまま開く」）とWindows SmartScreenの警告が出る場合があることをリリースノートに明記する。
11. ZimaOSでv2ストアURLから更新を確認し、必要なら旧クライアント向け `store/main.zip` でも追加・更新を確認する。
12. 配布された `.dmg` を macOS 実機でマウントし、Applications フォルダへの配置および起動ができることを確認する。
13. GitHub ReleaseからApple Silicon／IntelのDMGをmacOS実機（可能なら両アーキテクチャ）へダウンロードし、`SHA256SUMS`と照合したうえで手動インストール・初回起動を確認する。macOSアプリはad-hoc署名のみで公証されていないため、Gatekeeperの「このまま開く」確認が出ること、および確認後に起動できることを実機で確かめる。続けて旧バージョンから「設定 ▸ アップデート」で新バージョンを検出し、署名検証付きでダウンロード・インストール・再起動できることを確認する。
14. Windows 10/11 x64 実機で、NSISインストーラーを標準ユーザーとして導入し、初回起動、WebView2、SmartScreen警告、Windows Credential Manager、ターミナル（Git Bashあり／なし）、Chromiumの初回インストール、日本語ユーザー名のワークスペース、アンインストールを確認する。MSIは英語版であることを確認する。
15. Windows実機で`SHA256SUMS`と`-Windows-x64-Setup.exe`／`-Windows-x64.msi`のチェックサムを照合する。
16. Linux x64 実機で AppImage と deb を導入し、初回起動、OS Keychain（Secret Service）、ターミナル、Chromiumの初回インストール、デスクトップエントリ、アンインストールを確認する。AppImageではアプリ内更新、debではGitHub Releasesからの手動更新を確認する。
17. Linux実機で`SHA256SUMS`と`-Linux-x64.AppImage`／`-Linux-x64.deb`のチェックサムを照合する。

公開前にチェックする点:

- `LICENSE`、`NOTICE` とアイコンのライセンス情報が残っている。
- API key、パスワード、個人データ、開発機の生成物が含まれていない。`scripts/check-secrets.py` と `repo-hygiene` ジョブが成功している。
- `dependency-audit` が成功し、Python 依存の CycloneDX SBOM（`sbom-python` artifact）が生成されている。`dependency-audit-rust` も成功し、`Cargo.lock` に既知の脆弱性がない。
- 設定ファイル・README・Releaseノートに非公開の作業用リポジトリへの参照が含まれていない。
- ZimaOSの既存手動導入からストア導入へのデータ引き継ぎは自動ではない。ガイドのバックアップ・復元説明を維持する。
- `apps/MIX-agent/docker-compose.yml`の`x-casaos.version`がReleaseタグと一致している。v2/v1ストアは`scripts/stage-store-source.sh`で同じファイルから生成する。
- Desktop版は署名付き自動更新を提供する。秘密鍵はRepository Secretsの`TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`のみに置き、リポジトリや生成物へ含めない。公開鍵は`tauri.conf.json`に埋め込む。鍵のオフラインバックアップをリポジトリ外に保管する。
- `latest.json`の4プラットフォーム（`darwin-aarch64` / `darwin-x86_64` / `windows-x86_64` / `linux-x86_64`）すべてにURLと`.sig`の内容が入っていることを確認する。1つでも欠けるとTauriはマニフェスト全体を無効とみなす。
- `desktop-release`の`Verify tag matches app version`が成功し、タグと各manifestのバージョンが一致している。
- `Release notes are present and publishable`（`scripts/check-release-notes.mjs`）が成功している。`docs/release-notes-vX.Y.Z.md` が無い、または「未公開ドラフト」と書かれたままのタグは公開できません。
- `desktop-rust (windows-latest)`と`desktop-package-windows`が成功し、NSIS/MSIのビルドが通っている。WindowsのPTYテスト2件は既知のConPTY制約として`#[cfg_attr(windows, ignore)]`でスキップされるため、Windows実機で手動確認する（`docs/verification.md`参照）。
- `apps/desktop/browser/src/chromium-manifest.json` の `version` / `sha256` / `size` が、実際に配布する Chrome for Testing の zip と一致している。`pnpm chromium:pin` の差分をレビューし、リリース前に各プラットフォームで初回インストールを確認する。`desktop-release` の `verify-chromium-pin` が成功していること。
- 初回インストールが失敗した場合に、システムの Chromium / Chrome / Edge へフォールバックできる。オフライン環境向けの手動配置手順がリリースノートにある。

### Developer ID / Authenticode 署名を有効化する場合（証明書未登録）

現状は macOS が ad-hoc 署名のみ、Windows が未署名です。証明書を用意できた時点で、以下を一度に切り替えます。証明書・パスワードは Repository Secrets のみに置き、リポジトリへコミットしないでください。

1. Apple Developer Program に加入し、Developer ID Application 証明書と App Store Connect API キー（または Apple ID の app-specific password）を用意する。
2. Secrets に `APPLE_CERTIFICATE` / `APPLE_CERTIFICATE_PASSWORD` / `APPLE_SIGNING_IDENTITY` / `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` を登録する。登録すると`Configure optional Apple signing` がビルドへ資格情報を渡し、署名・公証・stapleの検証まで自動で切り替わる。
3. 証明書が無い間は ad-hoc 署名のままなので、`Verify macOS signature and bundle architecture` が `Signature=adhoc` を検証する分岐を維持する。
4. Windows は Authenticode 証明書（EV 推奨）を Secrets に登録し、Tauri の `certificateThumbprint` / `digestAlgorithm` / `timestampUrl` を設定して NSIS/MSI に署名する。署名後に `signtool verify /pa /v` を実行する。
5. 署名済み成果物で Gatekeeper / SmartScreen の警告が出ないこと、自動更新の `.sig` が壊れていないことを実機で確認する。

証明書が無い間は、リリースノートに「ad-hoc 署名 / 未署名」と初回確認手順を明記する運用を続けます。
