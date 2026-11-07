# Desktop UX ベースライン（2026-11-06）

> 状態: 計測実装済み / ユーザーテスト待ち
> 対象: `apps/desktop/ui`
> 関連: [`ux-improvement-plan.md`](ux-improvement-plan.md)

---

## 1. 目的

「UXが悪い」を主観のまま直さないために、改善前の状態を記録する。
Phase 0 の成果物は次の2つ。

1. 5つの代表ジャーニーの成功率・所要時間・離脱点
2. アプリ内で記録するローカルイベントのベースライン

ユーザーテストはまだ実施していない。この文書は**計測の準備ができた状態**と、
コードから確認できる初期所見を残す。

---

## 2. 代表ジャーニー

| # | ジャーニー | 開始 | 完了条件 | 主な計測 |
| :-- | :--- | :--- | :--- | :--- |
| J1 | 初回起動 → 最初の応答 | アプリ起動 | アシスタントの最初のトークン | 所要時間、クリック数、離脱点 |
| J2 | 新規スレッド → 送信 | 「新規」を押す | メッセージ送信 | クリック数、迷った箇所 |
| J3 | 承認待ちの解決 | 承認バナー表示 | 許可 / 拒否 | 会話からの離脱、所要時間 |
| J4 | 設定変更 → 保存 | 設定を開く | 保存完了 | タブ到達、保存方式の誤解 |
| J5 | 過去スレッドの検索 → 開く | 検索を開く | スレッド表示 | 検索手段の発見、0件率 |

目標値は [`ux-improvement-plan.md`](ux-improvement-plan.md) の「成功指標」を参照。

---

## 3. ローカルイベント

外部には送信しない。`localStorage` の `mix-agent-ux-events` に
イベント名ごとの回数と最終発生時刻だけを保存する。
プロンプト、メッセージ本文、ファイルパス、引数は保存しない。

| イベント | 記録場所 | 意味 |
| :--- | :--- | :--- |
| `new_thread` | サイドバー / パレット / メニュー | 新しいスレッドを開始した |
| `message_sent` | `ChatPane` | 送信が成功した（面 / モード / 一時モード） |
| `run_completed` | `ChatPane` | Run が終端状態になった |
| `approval_shown` | `ChatPane` | 承認待ちが表示された（面 / ツール） |
| `approval_decided` | バナー / カード / 受信箱 | 承認を判断した（場所 / 許可 / 記憶） |
| `surface_switched` | `SurfaceSwitcher` | チャット ↔ コーディングを切り替えた |
| `settings_opened` | UI ストア | 設定タブを開いた |
| `settings_saved` | `SettingsPane` | 設定を保存した |
| `settings_save_failed` | `SettingsPane` | 設定の保存に失敗した |
| `search_opened` | UI ストア | 検索 / コマンドパレットを開いた |
| `panel_opened` | `RightPanel` | 右パネルを開いた（種類 / 面） |
| `error_shown` | `ChatPane` | 送信エラーが表示された |

### 見方

設定 ▸ 診断 ▸ **UX イベント（この端末のみ）** に回数と最終時刻が出る。
「計測をクリア」で消せる。開発時は DevTools で
`localStorage.getItem("mix-agent-ux-events")` を読んでもよい。

---

## 4. 初期所見（コードから確認できるもの）

ユーザーテスト前の時点で、構造的に摩擦が予想される箇所。

| 所見 | 根拠 | 対応する Phase |
| :--- | :--- | :--- |
| 承認の入口が3箇所に分散し、ヘッダーから設定へ飛ぶ | `ApprovalBanner` / `ApprovalCard` / `App.tsx` の `ApprovalIndicator` | Phase 1 / 3（ドロワー実装済み） |
| 検索が広い画面でしか見えない | `App.tsx` の `md:flex` | Phase 1（実装済み） |
| タイトルバーにアイコンが密集 | `App.tsx` のヘッダー | Phase 1（実装済み） |
| 本文 13px / メタ 11px が小さい | `tokens.css` | Phase 1（実装済み） |
| 面の切り替えがドロップダウンで、他方の履歴が見えない | `SurfaceSwitcher` / `Sidebar` | Phase 1（実装済み） |
| 設定が17タブ、保存方式が混在 | `SettingsPane` | Phase 4（6グループ再編のみ実装済み） |
| 実行状態がサイドバー / パネル / transcript に分散 | `Sidebar` / `ActivityPanel` / `LiveBlock` | Phase 3（ステータスバー実装済み） |
| 初回導線が Base URL / API キー中心 | `Onboarding` / `StepProvider` | Phase 4 |

---

## 5. 次にやること

1. 5名（または社内ドッグフーディング3名以上）で J1〜J5 を実施する。
2. 各ジャーニーの所要時間・クリック数・「どこを押すか分からない」発言を記録する。
3. この文書の表にベースラインを追記する。
4. Phase 1 の変更後、同じ手順で再測定する。

### 記録テンプレート

```text
日付:
参加者: （匿名ID）
環境: macOS / Windows / Linux, ウィンドウ幅
J1: 秒 / クリック / 離脱点
J2: 秒 / クリック / 離脱点
J3: 秒 / 離脱有無 / 迷った箇所
J4: 秒 / 保存方式の誤解
J5: 秒 / 検索手段 / 0件有無
自由記述:
```
