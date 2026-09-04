import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Field, Title, useRows } from "@/components/shared";
import { BookOpen } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

export function Skills() {
  const rows = useRows("/skills"), qc = useQueryClient();
  const [editing, setEditing] = useState<Row | null>(null);
  const [error, setError] = useState<unknown>(null);
  return <main className="page">
    <Title title="Skills" sub="検証済みの作業手順を保存して、次の会話で再利用します。" />
    <ErrorBox error={error || rows.error} />
    <form className="card" key={editing?.id || "new"} onSubmit={async e => {
      e.preventDefault(); const f = new FormData(e.currentTarget);
      try { await api("/skills" + (editing ? "/" + editing.id : ""), editing ? "PATCH" : "POST", { name: f.get("name"), description: f.get("description"), content: f.get("content"), enabled: f.has("enabled") }); setEditing(null); qc.invalidateQueries({ queryKey: ["/skills"] }); }
      catch (err) { setError(err); }
    }}>
      <div className="form-grid"><Field label="名前"><input name="name" required defaultValue={editing?.data.name} /></Field><Field label="説明"><input name="description" defaultValue={editing?.data.description} /></Field></div>
      <Field label="手順"><textarea name="content" rows={7} required defaultValue={editing?.data.content} placeholder="目的、前提、手順、確認方法を簡潔に記載" /></Field>
      <label className="check"><input name="enabled" type="checkbox" defaultChecked={editing ? editing.data.enabled !== false : true} />会話で利用する</label>
      <div className="form-actions"><Button>{editing ? "更新" : "保存"}</Button>{editing && <Button type="button" variant="ghost" onClick={() => setEditing(null)}>キャンセル</Button>}</div>
    </form>
    {rows.data?.map(skill => <div className={"card memory-card " + (skill.data.deleted ? "deleted" : "")} key={skill.id}>
      <BookOpen size={20} /><div className="grow"><h3>{skill.data.name}</h3><small>{skill.data.description || "説明なし"} · {skill.data.source_run ? "Agentが保存" : "手動で保存"}</small><p>{skill.data.content}</p></div>
      <div className="row-actions"><Button variant="ghost" onClick={() => setEditing(skill)}>編集</Button>{!skill.data.deleted && <Button variant="ghost" onClick={async () => { if (confirm("このSkillを削除しますか？履歴から復元できます。")) { try { await api("/skills/" + skill.id, "DELETE"); qc.invalidateQueries({ queryKey: ["/skills"] }); } catch (err) { setError(err); } } }}>削除</Button>}</div>
    </div>)}
  </main>;
}
