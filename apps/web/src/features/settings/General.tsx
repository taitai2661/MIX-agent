import { api } from "@/app/api";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { ErrorBox, Field, Title, useRows } from "@/components/shared";
import { AccentPicker } from "@/components/accent";
import { ThemeController } from "@/components/theme";
import { getLanguage, setLanguage, t, useLanguage } from "@/app/i18n";

export function General() {
  useLanguage();
  const setting = useQuery({
      queryKey: ["/settings"],
      queryFn: () => api("/settings"),
    }),
    models = useRows("/models"),
    qc = useQueryClient();
  const [error, setError] = useState<unknown>(null),
    [saved, setSaved] = useState(false);
  return (
    <>
      <Title
        title={t("基本設定")}
        sub={t("表示テーマ・表示言語と、会話で使う既定モデルを設定します。")}
      />
      <ErrorBox error={error || setting.error} />
      <section className="card theme-card">
        <div>
          <h3>{t("表示テーマ")}</h3>
          <p>{t("ライト、ダーク、またはOSの設定に合わせて表示を切り替えます。")}</p>
        </div>
        <ThemeController />
      </section>
      <section className="card theme-card">
        <div>
          <h3>{t("アクセントカラー")}</h3>
          <p>{t("ボタンやリンクなど、操作の目印となる色を選びます。")}</p>
        </div>
        <AccentPicker />
      </section>
      <section className="card theme-card"><div><h3>{t("表示言語")}</h3><p>{t("ブラウザーの言語から初期値を選びます。変更内容はアカウントに保存されます。")}</p></div><select aria-label={t("表示言語")} value={getLanguage()} onChange={async e => { const previous = getLanguage(); const value = e.target.value as "ja" | "en"; setLanguage(value); try { await api("/settings", "PUT", { ui_language: value }); await qc.invalidateQueries({ queryKey: ["/settings"] }); } catch (cause) { setLanguage(previous); setError(cause); } }}><option value="ja">{t("日本語")}</option><option value="en">English</option></select></section>
      {setting.data && (
        <form
          className="card"
          key={setting.dataUpdatedAt}
          onSubmit={async (e) => {
            e.preventDefault();
            const f = new FormData(e.currentTarget);
            try {
              await api("/settings", "PUT", {
                default_model_id: f.get("default_model_id"),
                setup_complete: setting.data.data.setup_complete,
              });
              setSaved(true);
              qc.invalidateQueries({ queryKey: ["/settings"] });
            } catch (e) {
              setError(e);
            }
          }}
        >
          <Field label={t("既定モデル")}>
            <select
              name="default_model_id"
              defaultValue={setting.data.data.default_model_id || ""}
            >
              <option value="">{t("未設定")}</option>
              <option value="auto">Auto</option>
              {models.data?.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.data.name || m.data.model_id}
                </option>
              ))}
            </select>
          </Field>
          <div className="notice">{t("外部サービスへの通信許可先は「通信とネットワーク」で設定します。")}</div>
          <Button>{t("保存")}</Button>
          {saved && <span className="success">{t("保存しました")}</span>}
        </form>
      )}
    </>
  );
}
