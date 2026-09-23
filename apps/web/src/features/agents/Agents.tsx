import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Bot, Plus } from "lucide-react";
import { useState } from "react";

import { Empty, ErrorBox, Field, Title, useRows } from "@/components/shared";

export function Agents() {
  const rows = useRows("/agents"),
    models = useRows("/models"),
    tools = useQuery<any[]>({
      queryKey: ["/tools"],
      queryFn: () => api("/tools"),
    }),
    skills = useRows("/skills"),
    qc = useQueryClient();
  const [editing, setEditing] = useState<Row | null>(null),
    [open, setOpen] = useState(false),
    [error, setError] = useState<unknown>(null);
  return (
    <main className="page">
      <Title
        title="アシスタント設定"
        sub="役割と使えるToolを決めて、自分に合ったアシスタントに。"
        action={
          <Button
            onClick={() => {
              setEditing(null);
              setOpen(true);
            }}
          >
            <Plus size={16} />
            設定を追加
          </Button>
        }
      />
      <ErrorBox error={error || rows.error} />
      {open && (
        <form
          className="card"
          key={editing?.id || "new"}
          onSubmit={async (e) => {
            e.preventDefault();
            const f = new FormData(e.currentTarget);
            try {
              await api(
                "/agents" + (editing ? "/" + editing.id : ""),
                editing ? "PATCH" : "POST",
                {
                  name: f.get("name"),
                  system_prompt: f.get("system_prompt"),
                  model_id: f.get("model_id"),
                  mode: f.get("mode"),
                  tool_ids: Array.from(
                    new Set([
                      ...f.getAll("tools").map(String),
                      ...(f.has("auto_learn")
                        ? [
                            "memory_search",
                            "memory_add",
                            "memory_update",
                            "skill_search",
                            "skill_add",
                            "skill_update",
                          ]
                        : []),
                    ])),
                  memory_scopes: f.has("memory") ? ["user"] : [],
                  skill_ids: f.getAll("skills"),
                  auto_learn: f.has("auto_learn"),
                  max_steps: Number(f.get("steps")),
                  max_seconds: Number(f.get("seconds")),
                  max_tool_calls: Number(f.get("calls")),
                  model_settings: {
                    max_output_tokens: Number(f.get("tokens")),
                    ...(f.get("temperature") !== ""
                      ? { temperature: Number(f.get("temperature")) }
                      : {}),
                  },
                },
              );
              setOpen(false);
              qc.invalidateQueries({ queryKey: ["/agents"] });
            } catch (e) {
              setError(e);
            }
          }}
        >
          <div className="form-grid">
            <Field label="名前">
              <input name="name" required defaultValue={editing?.data.name} />
            </Field>
            <Field label="モデル">
              <select name="model_id" defaultValue={editing?.data.model_id}>
                <option value="auto">Auto</option>
                {models.data?.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.data.name || m.data.model_id}
                  </option>
                ))}
              </select>
            </Field>
            <Field label="モード">
              <select name="mode" defaultValue={editing?.data.mode || "agent"}>
                <option value="chat">chat</option>
                <option value="thinking">thinking</option>
                <option value="agent">agent</option>
              </select>
              <small>chat と thinking は固定予算です。下の実行予算は agent で使用します。</small>
            </Field>
            <Field label="最大モデル呼び出し回数">
              <input
                name="steps"
                type="number"
                min="1"
                max="2000"
                defaultValue={editing?.data.max_steps || 200}
              />
            </Field>
            <Field label="最大実行時間（秒）">
              <input
                name="seconds"
                type="number"
                min="30"
                max="86400"
                defaultValue={editing?.data.max_seconds || 3600}
              />
            </Field>
            <Field label="最大Tool Call数">
              <input
                name="calls"
                type="number"
                min="1"
                max="5000"
                defaultValue={editing?.data.max_tool_calls || 500}
              />
            </Field>
            <Field label="最大出力Token">
              <input
                name="tokens"
                type="number"
                min="1025"
                max="32000"
                defaultValue={
                  editing?.data.model_settings?.max_output_tokens || 4096
                }
              />
            </Field>
            <Field label="Temperature（互換API向け）">
              <input
                name="temperature"
                type="number"
                step="0.1"
                min="0"
                max="2"
                defaultValue={editing?.data.model_settings?.temperature}
              />
            </Field>
          </div>
          <Field label="System Prompt">
            <textarea
              name="system_prompt"
              rows={5}
              defaultValue={
                editing?.data.system_prompt ||
                "あなたは親切で正確なアシスタントです。"
              }
            />
          </Field>
          <label className="check">
            <input
              name="memory"
              type="checkbox"
              defaultChecked={
                editing ? editing.data.memory_scopes?.length > 0 : true
              }
            />
            ユーザーMemoryを使用する
          </label>
          <label className="check">
            <input name="auto_learn" type="checkbox" defaultChecked={editing ? editing.data.auto_learn !== false : true} />
            会話からMemory・Skillを自動で学習する
          </label>
          <h3>使用可能Tool</h3>
          <div className="tool-checks">
            {tools.data?.map((t) => (
              <label className="check" key={t.id}>
                <input
                  name="tools"
                  value={t.id}
                  type="checkbox"
                  defaultChecked={editing?.data.tool_ids?.includes(t.id)}
                />
                <span>
                  {t.model_name}
                  <small>{t.source}</small>
                </span>
              </label>
            ))}
          </div>
          <h3>使用するSkill</h3>
          <div className="tool-checks">
            {skills.data?.map((skill) => (
              <label className="check" key={skill.id}>
                <input name="skills" value={skill.id} type="checkbox" defaultChecked={editing?.data.skill_ids?.includes(skill.id)} />
                <span>{skill.data.name}<small>{skill.data.description || "再利用可能な手順"}</small></span>
              </label>
            ))}
          </div>
          <div className="form-actions">
            <Button>保存</Button>
            <Button
              variant="ghost"
              type="button"
              onClick={() => setOpen(false)}
            >
              キャンセル
            </Button>
          </div>
        </form>
      )}
      <div className="agent-grid">
        {rows.data?.map((a) => (
          <button
            className="card agent-card"
            key={a.id}
            onClick={() => {
              setEditing(a);
              setOpen(true);
            }}
          >
            <span className="agent-icon">
              <Bot size={25} />
            </span>
            <h3>{a.data.name}</h3>
            <p>{a.data.system_prompt?.slice(0, 100)}</p>
            <div>
              <span className="tag">{a.data.mode}</span>
              <span>{a.data.tool_ids.length} tools</span>
            </div>
          </button>
        ))}
      </div>
      {!rows.data?.length && !open && (
        <Empty>
          用途別のアシスタント設定を作成できます。標準のAgentモードは設定を作成せずに利用できます。
        </Empty>
      )}
    </main>
  );
}
