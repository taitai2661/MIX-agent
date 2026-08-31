import { api } from "@/app/api";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { ErrorBox, Field, Title, useRows } from "@/components/shared";

export function Tools() {
  const tools = useQuery<any[]>({
      queryKey: ["/tools"],
      queryFn: () => api("/tools"),
    }),
    rules = useRows("/permission-rules"),
    agents = useRows("/agents"),
    qc = useQueryClient();
  const [agent, setAgent] = useState(""),
    [error, setError] = useState<unknown>(null);
  return (
    <>
      <Title
        title="Tools・権限"
        sub="AIが使える能力と、実行時の確認方法を管理します。"
      />
      <div className="notice">
        Terminalは専用workspace全体を操作できます。ホスト・Docker
        Socketへのアクセスは提供しません。
      </div>
      <Field label="権限を設定するAgent">
        <select value={agent} onChange={(e) => setAgent(e.target.value)}>
          <option value="">標準Agent</option>
          {agents.data?.map((a) => (
            <option key={a.id} value={a.id}>
              {a.data.name}
            </option>
          ))}
        </select>
      </Field>
      <ErrorBox error={error || tools.error} />
      {tools.data?.map((t) => {
        const rule = rules.data
          ?.filter((r) => r.data.agent_id === agent && r.data.tool_id === t.id)
          .at(-1);
        return (
          <div className="tool-row" key={t.id}>
            <div>
              <b>{t.model_name}</b>
              <p>{t.description}</p>
              <small>
                {t.source} · {t.risk}
              </small>
            </div>
            <select
              aria-label={t.model_name + "の権限"}
              value={rule?.data.permission || t.default_permission}
              onChange={async (e) => {
                try {
                  await api("/permission-rules", "POST", {
                    agent_id: agent,
                    tool_id: t.id,
                    permission: e.target.value,
                  });
                  qc.invalidateQueries({ queryKey: ["/permission-rules"] });
                } catch (e) {
                  setError(e);
                }
              }}
            >
              <option value="allow">Always Allow</option>
              <option value="ask">Ask</option>
              <option value="deny">Deny</option>
            </select>
          </div>
        );
      })}
    </>
  );
}
