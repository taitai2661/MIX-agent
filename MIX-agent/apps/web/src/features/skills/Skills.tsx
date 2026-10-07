import { t } from "@/app/i18n";
import { api, binary, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Field, Title, useRows } from "@/components/shared";
import { BookOpen, Download, FileArchive, Upload } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

interface ResourceView {
  id: string;
  path: string;
  content: string;
  binary: boolean;
}

export function Skills() {
  const rows = useRows("/skills"), qc = useQueryClient();
  const [editing, setEditing] = useState<Row | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<ResourceView | null>(null);
  const reload = () => qc.invalidateQueries({ queryKey: ["/skills"] });

  const download = async (path: string, filename: string) => {
    try {
      const response = await fetch("/api/v1" + path, { credentials: "same-origin" });
      if (!response.ok) throw new Error(t("書き出しに失敗しました"));
      const url = URL.createObjectURL(await response.blob());
      const link = document.createElement("a");
      link.href = url;
      link.download = filename;
      link.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (err) {
      setError(err);
    }
  };

  const importSkill = async (form: HTMLFormElement) => {
    setError(null);
    setBusy(true);
    setNotice("");
    const data = new FormData(form);
    const archive = data.get("archive");
    try {
      if (archive instanceof File && archive.size > 0) {
        const body = new FormData();
        body.append("file", archive);
        const response = await binary("/skills/import/archive", body);
        const imported = await response.json();
        setNotice(t("Skillを取り込みました") + " (" + imported.length + ")");
      } else {
        await api("/skills/import", "POST", { text: data.get("text") });
        setNotice(t("Skillを取り込みました"));
      }
      form.reset();
      reload();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  };

  return <main className="page">
    <Title title="Skills" sub={t("Agent Skills（SKILL.md）の取り込み・書き出しと、検証済み手順の再利用。")} />
    <ErrorBox error={error || rows.error} />
    {notice && <p className="success">{notice}</p>}
    <form className="card" onSubmit={e => { e.preventDefault(); importSkill(e.currentTarget); }}>
      <h3><Upload size={18} />{t("Agent Skillを取り込む")}</h3>
      <Field label={t("SKILL.mdを貼り付け")}><textarea name="text" rows={6} placeholder={"---\nname: my-skill\ndescription: いつ使うかを具体的に記載\n---"} /></Field>
      <Field label={t("またはzipをアップロード")} hint={t("scripts・references・assetsは保存のみで実行しません。")}><input name="archive" type="file" accept=".zip" /></Field>
      <div className="form-actions">
        <Button disabled={busy}>{t("取り込む")}</Button>
        <Button type="button" variant="ghost" disabled={busy} onClick={async () => {
          setError(null); setBusy(true); setNotice("");
          try { const result = await api<any>("/skills/import/directory", "POST", {}); setNotice(t("サーバーディレクトリから取り込みました") + ": " + result.imported); reload(); }
          catch (err) { setError(err); } finally { setBusy(false); }
        }}>{t("サーバーディレクトリから取込")}</Button>
      </div>
    </form>
    <form className="card" key={editing?.id || "new"} onSubmit={async e => {
      e.preventDefault(); const f = new FormData(e.currentTarget);
      try { await api("/skills" + (editing ? "/" + editing.id : ""), editing ? "PATCH" : "POST", { name: f.get("name"), description: f.get("description"), content: f.get("content"), enabled: f.has("enabled") }); setEditing(null); reload(); }
      catch (err) { setError(err); }
    }}>
      <div className="form-grid"><Field label={t("名前")}><input name="name" required defaultValue={editing?.data.name} /></Field><Field label={t("説明")}><input name="description" defaultValue={editing?.data.description} /></Field></div>
      <Field label={t("手順")}><textarea name="content" rows={7} required defaultValue={editing?.data.content} placeholder={t("目的、前提、手順、確認方法を簡潔に記載")} /></Field>
      <label className="check"><input name="enabled" type="checkbox" defaultChecked={editing ? editing.data.enabled !== false : true} />{t("会話で利用する")}</label>
      <div className="form-actions"><Button>{editing ? t("更新") : t("保存")}</Button>{editing && <Button type="button" variant="ghost" onClick={() => setEditing(null)}>{t("キャンセル")}</Button>}</div>
    </form>
    {rows.data?.map(skill => {
      const isAgent = skill.data.format === "agent-skill";
      const files = Object.keys(skill.data.files || {});
      return <div className={"card memory-card " + (skill.data.deleted ? "deleted" : "")} key={skill.id}>
        <BookOpen size={20} /><div className="grow">
          <h3>{skill.data.name}</h3>
          <small>{skill.data.description || t("説明なし")} · {skill.data.source_run ? t("Agentが保存") : t("手動で保存")}{isAgent ? " · Agent Skill" : ""}</small>
          <p>{skill.data.content}</p>
          {isAgent && <p className="tags">
            {skill.data.license && <span className="tag">{skill.data.license}</span>}
            {skill.data.compatibility && <span className="tag">{skill.data.compatibility}</span>}
            {(skill.data.allowed_tools || []).map((tool: string) => <span className="tag" key={tool}>{tool}</span>)}
          </p>}
          {files.length > 0 && <ul className="file-list">{files.map(name => <li key={name}>
            <button type="button" className="link" onClick={async () => {
              try { const r = await api<any>("/skills/" + skill.id + "/resources?path=" + encodeURIComponent(name)); setPreview({ id: skill.id, path: name, content: r.content, binary: r.binary }); }
              catch (err) { setError(err); }
            }}>{name}</button>
          </li>)}</ul>}
          {preview?.id === skill.id && <div className="resource-view">
            <small>{preview.path}{preview.binary ? " · " + t("バイナリ（表示のみ）") : ""}</small>
            <pre>{preview.binary ? t("バイナリファイルは表示できません。") : preview.content}</pre>
          </div>}
        </div>
        <div className="row-actions">
          <Button variant="ghost" onClick={() => download("/skills/" + skill.id + "/export", skill.data.name + ".SKILL.md")}><Download size={15} />SKILL.md</Button>
          {files.length > 0 && <Button variant="ghost" onClick={() => download("/skills/" + skill.id + "/export.zip", skill.data.name + ".zip")}><FileArchive size={15} />zip</Button>}
          <Button variant="ghost" onClick={() => setEditing(skill)}>{t("編集")}</Button>
          {!skill.data.deleted && <Button variant="ghost" onClick={async () => { if (confirm(t("このSkillを削除しますか？履歴から復元できます。"))) { try { await api("/skills/" + skill.id, "DELETE"); reload(); } catch (err) { setError(err); } } }}>{t("削除")}</Button>}
        </div>
      </div>;
    })}
  </main>;
}
