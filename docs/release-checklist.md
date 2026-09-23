# v0.2.0 公開手順

この文書は公開前の作業確認用です。初回公開時は順番に実施します。

1. リポジトリの変更をレビューし、PRの `Validate release candidate` が成功したことを確認する。
2. 実機または検証用Docker環境で新規導入を試し、管理者作成、Provider接続、Runner、MCPを確認する。
3. 既存データの暗号化バックアップを取得し、検証環境へ復元する。DB migration後の会話と設定を確認する。
4. 公開リポジトリのmainへ反映し、`v0.2.0` タグを作成する。タグの `Release MIX-agent` が8種類のAMD64イメージをGHCRへ配布する。
5. GitHub Packagesで8種類のパッケージをPublicに変更する。匿名で各イメージをpullできることを確認する。
6. Actionsの `Publish store and GitHub Release` に `v0.2.0` を指定して実行する。生成された `store.json`、ZimaOSからの新規導入、GitHub Releaseの内容を確認する。

公開前にチェックする点:

- `LICENSE`、`NOTICE` とアイコンのMITライセンスが残っている。
- API key、パスワード、個人データ、開発機の生成物が含まれていない。
- 設定ファイル・README・Releaseノートに非公開の作業用リポジトリへの参照が含まれていない。
- ZimaOSの既存手動導入からストア導入へのデータ引き継ぎは自動ではない。ガイドのバックアップ・復元説明を維持する。
