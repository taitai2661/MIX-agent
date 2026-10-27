# アップデート署名鍵の運用

Desktop 版の自動更新は Tauri updater の署名を必須とします。秘密鍵が無い、または公開鍵と一致しないビルドはクライアントに拒否されます。

## 鍵の保管場所

| 用途 | 場所 |
| --- | --- |
| CI がビルド時に使う秘密鍵 | GitHub Actions Repository Secrets `TAURI_SIGNING_PRIVATE_KEY` |
| 秘密鍵のパスワード | GitHub Actions Repository Secrets `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |
| オフラインバックアップ | リポジトリ外（この開発機ではデスクトップ上の `MIX-agent-keys/`） |
| 公開鍵 | `apps/desktop/app/tauri.conf.json` の `plugins.updater.pubkey` |

秘密鍵・パスワードをリポジトリ、Issue、ログ、Release に含めないでください。`.gitignore` は `*.key` / `*.key.pub` を除外しますが、置き場所の保証ではありません。

## 新規生成

```bash
cd apps/desktop
pnpm tauri signer generate -w ~/Desktop/MIX-agent-keys/tauri-updater.key
```

パスワードを設定した場合、その値も Secrets に登録します。

```bash
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo taitai2661/MIX-agent \
  --body "$(cat ~/Desktop/MIX-agent-keys/tauri-updater.key)"
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo taitai2661/MIX-agent \
  --body "$(cat ~/Desktop/MIX-agent-keys/TAURI_SIGNING_PRIVATE_KEY_PASSWORD.txt)"
```

`tauri-updater.key.pub` の内容を `tauri.conf.json` の `pubkey` に貼り付けます（ファイルパスは不可）。

## ローカルでリリースビルドする場合

`createUpdaterArtifacts: true` のため、`tauri build` には署名鍵が必要です。

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/Desktop/MIX-agent-keys/tauri-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat ~/Desktop/MIX-agent-keys/TAURI_SIGNING_PRIVATE_KEY_PASSWORD.txt)"
pnpm --filter mix-agent-desktop-workspace tauri:build
```

`--no-bundle`（CI の smoke test）はバンドルを作らないため鍵不要です。

## ローテーション

既存ユーザーのアプリは**インストール済みの公開鍵**でしか新バージョンを検証できません。鍵を差し替える場合は次の順序が必要です。

1. 旧鍵のまま、新公開鍵を埋め込んだバージョンをリリースする。
2. そのバージョンが十分に行き渡ってから、CI の秘密鍵を新鍵へ切り替える。

旧鍵を失った場合、既存インストールは自動更新を受け取れないため、手動ダウンロードへの移行案内が必要です。

## 検証

- CI の `Latest.json` 生成ステップが 4 プラットフォーム分の URL と `.sig` を検証します（`scripts/build-latest-json.mjs`）。
- Release 後、旧バージョンから「設定 ▸ アップデート」で検出 → ダウンロード → 再起動を各 OS 実機で確認します。
- Linux の `.deb` はアプリ内更新の対象外です。AppImage のみ自動更新できます。
