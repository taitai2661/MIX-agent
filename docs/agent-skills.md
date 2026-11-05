# Agent Skills 対応

MIX-agent は [Agent Skills 仕様](https://agentskills.io/specification) の `SKILL.md` を
Desktop 版・Server 版の両方で扱います。このページは対応範囲、探索パス、保存形式、
セキュリティ上の境界をまとめたものです。

## 対応範囲

| 機能 | Desktop | Server |
|---|---|---|
| YAML frontmatter（`name` / `description`） | ✅ | ✅ |
| `license` / `compatibility` / `metadata` | ✅ 保存・往復 | ✅ 保存・往復 |
| `allowed-tools`（スペース区切り文字列） | ✅ | ✅ |
| プロジェクト Skills 探索 | ✅ | ✅ |
| ユーザー全体 Skills 探索 | ✅（設定で OFF 可） | ✅ |
| `scripts/` / `references/` / `assets/` | ✅ 一覧・読み取り | ✅ 一覧・読み取り |
| バイナリ同梱ファイル | ✅ base64 | ✅ base64 |
| zip / ディレクトリのインポート・エクスポート | ✅ | ✅ |
| スクリプトの自動実行 | ❌ しません | ❌ しません |
| `allowed-tools` の自動承認 | ❌ 表示のみ | ❌ 表示のみ |

## 探索パスと優先順位

### プロジェクト

プロジェクト直下の次のディレクトリを、**最初に存在したものだけ**読みます。

1. `.mix/skills/`
2. `.claude/skills/`
3. `.agents/skills/`

3 つをマージしないのは、リポジトリが意図せず他人の Skill を提供してしまうのを
防ぐためです。

### ユーザー全体

ホームディレクトリ直下の同じ 3 パスを、**最初に存在したものだけ**読みます。
Desktop 版は「設定 ▸ Skills ▸ ホームディレクトリの Skills を読み込む」で
無効化できます（既定は有効）。Server 版は `MIX_SKILLS_DIR` も追加で探索します。

### 同名の解決

```
プロジェクト Skill > ユーザー Skill > 保存済み Skill
```

最初に見つかったものを採用し、衝突はログに残します。ディレクトリの列挙順に
依存しない決定的な解決です。

## `SKILL.md` の形式

```markdown
---
name: pdf-processing
description: Extract PDF text, fill forms, merge files. Use when handling PDFs.
license: Apache-2.0
compatibility: Requires python3
metadata:
  author: example-org
  version: "1.0"
allowed-tools: Read Write Bash(git:*)
---

# PDF processing

1. …
```

- `name`: 1〜64 文字。小文字英数字と単一ハイフン。**親ディレクトリ名と一致**する必要があります。
- `description`: 1〜1024 文字。必須。
- `license`: 任意。短いライセンス名または同梱 LICENSE ファイル名。
- `compatibility`: 任意。1〜500 文字。
- `metadata`: 任意。文字列キー → 文字列値のマップ。
- `allowed-tools`: 任意。**スペース区切り文字列**。MIX では表示のみで、自動承認には使いません。

旧バージョンの MIX が出力した `allowed-tools: [a, b]` 形式も読み込み時は受け付けますが、
書き出しは必ず仕様形式になります。

Desktop 版は frontmatter のない Markdown も「旧形式」として読み込みますが、
新規インポートでは frontmatter を要求します。

> [!NOTE]
> Skill 本文はユーザーが導入した手順ですが、モデルへの入力としては信頼境界を越えた
> データとして扱われます。システムプロンプトの安全ポリシーが常に優先され、
> Skill の記述が承認を省略したり、秘密情報の保存を許可したりすることはありません。

## 同梱ファイル

`SKILL.md` と同じディレクトリにあるファイルを再帰的に収集します。

- 返すのは**相対パスの一覧**だけで、本文は `skill_resource` ツールで必要時に読みます。
- テキストはそのまま、バイナリは base64 と `binary: true` で保持します。
- 上限: 1 ファイル 256 KiB / 合計 2 MiB / 100 ファイル。
- `.git` / `node_modules` / `__pycache__` / `__MACOSX` などのメタデータやキャッシュは同梱しません。
- symlink は追跡しません。`..` を含むパスは拒否します。

### スクリプトの方針

`scripts/` のプログラムを**自動実行することはありません**。エージェントは
`skill_resource` で内容を読めますが、実行する場合は通常の `terminal` ツールと
ユーザー承認を経由します。これは「取り込んだ Skill を信頼済みコードとして
無条件に実行しない」という既存のセキュリティ境界に合わせた判断です。

## 保存とバックアップ

Desktop 版は SQLite の `skills` テーブルに JSON で保存します。

```json
{
  "name": "pdf-processing",
  "description": "…",
  "bodyMarkdown": "…",
  "license": "Apache-2.0",
  "compatibility": "…",
  "metadata": {"author": "example-org"},
  "allowedTools": ["Read", "Write"],
  "resources": [
    {"path": "scripts/run.py", "content": "…", "binary": false, "size": 1234}
  ]
}
```

Server 版は `skills` テーブルの `data` に同じ形で保存します。既存の Skills は
新フィールドが欠けていても既定値で読み込まれるため、マイグレーションは不要です。

## 検証とテスト

仕様の共有フィクスチャは `tests/fixtures/skills/` にあります。

- `tests/unit/test_skill_fixtures.py`（Server）
- `apps/desktop/core/src/skills/discovery.rs` の `the_shared_fixtures_match_the_expected_catalogue`（Desktop）

同じ `expected.json` を両方が読むため、片方だけのパーサ変更は CI で検出されます。
