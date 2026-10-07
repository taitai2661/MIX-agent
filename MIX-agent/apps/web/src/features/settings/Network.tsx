import { api } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Field, Title } from "@/components/shared";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { t } from "@/app/i18n";

export function NetworkSettings() {
  const setting = useQuery({
      queryKey: ["/settings"],
      queryFn: () => api("/settings"),
    }),
    qc = useQueryClient();
  const [error, setError] = useState<unknown>(null),
    [saved, setSaved] = useState(false);
  if (!setting.data) return <ErrorBox error={setting.error} />;
  return (
    <>
      <Title
        title={t("通信とネットワーク")}
        sub={t("Terminal・Browser・MCPが接続できる宛先を制限します。")}
      />
      <ErrorBox error={error || setting.error} />
      <form
        className="card"
        key={setting.dataUpdatedAt}
        onSubmit={async (e) => {
          e.preventDefault();
          const f = new FormData(e.currentTarget);
          try {
            await api("/settings", "PUT", {
              allowed_domains: String(f.get("domains"))
                .split("\n")
                .map((x) => x.trim())
                .filter(Boolean),
            });
            setSaved(true);
            qc.invalidateQueries({ queryKey: ["/settings"] });
          } catch (e) {
            setError(e);
          }
        }}
      >
        <Field
          label={t("Runner通信許可先（1行1ドメイン）")}
          hint={t("完全一致。サブドメインは別途指定します。未設定なら公開ドメインへの通信を許可します。入力すると指定ドメインだけに制限します。")}
        >
          <textarea
            name="domains"
            rows={8}
            defaultValue={setting.data.data.allowed_domains?.join("\n")}
            placeholder={"example.com\nregistry.npmjs.org"}
          />
        </Field>
        <div className="notice">
          {t("API Providerへの接続設定とは別です。Terminal・Browser・MCPの通信に適用します。LAN、localhost、metadata endpointは許可できません。")}
        </div>
        <Button>{t("保存")}</Button>
        {saved && <span className="success">{t("保存しました")}</span>}
      </form>
    </>
  );
}
