/** Local-runtime error codes shared with `apps/desktop/providers/src/local_error.rs`.
 *
 * The backend sends `mix-local-error:<code>|<json args>`; `translateBackendError`
 * in `errors.ts` turns it into the active language. Templates use the same
 * Japanese strings fed to `t()`, so `en.ts` only has to carry the translation.
 */

export interface LocalErrorSpec {
  /** Japanese template. Also the key looked up in `en.ts`. */
  template: string;
  /** Positional argument names, in the order the Rust side sends them. */
  params: readonly string[];
}

export const LOCAL_ERRORS: Record<string, LocalErrorSpec> = {
  start_unsupported: {
    template:
      "「{slug}」はアプリから起動できるローカルランタイムではありません。",
    params: ["slug"],
  },
  launcher_missing: {
    template:
      "{label} の起動コマンドが見つかりません。インストールと PATH を確認してください。",
    params: ["label"],
  },
  spawn_failed: {
    template: "{program} を起動できませんでした: {detail}",
    params: ["program", "detail"],
  },
  unresponsive: {
    template: "{label} は起動しましたが応答しません: {detail}",
    params: ["label", "detail"],
  },
  stop_external: {
    template:
      "このランタイムはアプリの外で起動されています。ターミナルなど起動した側で停止してください。",
    params: [],
  },
  cli_spawn_failed: {
    template: "{program} を実行できませんでした: {detail}",
    params: ["program", "detail"],
  },
  cli_failed: {
    template: "{program} が失敗しました: {detail}",
    params: ["program", "detail"],
  },
  pull_unsupported: {
    template: "このランタイムはモデルの取得に対応していません: {slug}",
    params: ["slug"],
  },
  delete_unsupported: {
    template: "このランタイムはモデルの削除に対応していません: {slug}",
    params: ["slug"],
  },
  load_unsupported: {
    template: "このランタイムはモデルのロードに対応していません: {slug}",
    params: ["slug"],
  },
  unload_unsupported: {
    template: "このランタイムはモデルのアンロードに対応していません: {slug}",
    params: ["slug"],
  },
  unknown_runtime: {
    template: "ローカルランタイムが見つかりません: {slug}",
    params: ["slug"],
  },
  model_name_empty: {
    template: "モデル名が空です。",
    params: [],
  },
  model_name_too_long: {
    template: "モデル名が長すぎます。",
    params: [],
  },
  model_name_invalid: {
    template: "モデル名に使用できない文字が含まれています。",
    params: [],
  },
  open_app_missing: {
    template: "アプリのパスが見つかりません: {path}",
    params: ["path"],
  },
  open_app_denied: {
    template: "アプリとして開けないパスです: {path}",
    params: ["path"],
  },
  request_failed: {
    template:
      "ローカルランタイムへのリクエストに失敗しました: {url} ({detail})",
    params: ["url", "detail"],
  },
  http_status: {
    template: "ローカルランタイムがエラーを返しました: {status} {detail}",
    params: ["status", "detail"],
  },
  invalid_json: {
    template:
      "ローカルランタイムから不正な JSON を受信しました: {url} ({detail})",
    params: ["url", "detail"],
  },
  pull_failed: {
    template: "モデルの取得に失敗しました: {detail}",
    params: ["detail"],
  },
  pull_cancelled: {
    template: "モデルの取得をキャンセルしました。",
    params: [],
  },
  lms_missing: {
    template:
      "LM Studio の CLI (`lms`) が見つかりません。LM Studio を起動し、`~/.lmstudio/bin/lms` の存在を確認してください。",
    params: [],
  },
  lms_spawn_failed: {
    template: "`lms` を実行できませんでした: {program} ({detail})",
    params: ["program", "detail"],
  },
  lms_failed: {
    template: "`lms {command}` が失敗しました: {detail}",
    params: ["command", "detail"],
  },
  probe_failed: {
    template: "ランタイムの状態を取得できませんでした: {endpoint} ({detail})",
    params: ["endpoint", "detail"],
  },
  health_failed: {
    template: "ランタイムのヘルスチェックに失敗しました。",
    params: [],
  },
};
