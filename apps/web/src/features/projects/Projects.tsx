import { t } from "@/app/i18n";
import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Title } from "@/components/shared";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type FormEvent } from "react";
import { Link, useParams } from "react-router-dom";

type Project = Row & { data: { name: string; instructions: string; sources: string[] } };
type Conversation = Row & { data: { title: string; project_id?: string | null } };

export function Projects() {
  const { id } = useParams();
  const qc = useQueryClient();
  const projects = useQuery<Project[]>({ queryKey: ["/projects"], queryFn: () => api("/projects") });
  const conversations = useQuery<Conversation[]>({ queryKey: ["/conversations", "projects"], queryFn: () => api("/conversations?state=active") });
  const [name, setName] = useState("");
  const [instructions, setInstructions] = useState("");
  const [source, setSource] = useState("");
  const [error, setError] = useState<unknown>(null);
  const project = projects.data?.find((item) => item.id === id);
  useEffect(() => { setInstructions(project?.data.instructions || ""); }, [project?.id, project?.data.instructions]);
  async function create(event: FormEvent) {
    event.preventDefault();
    if (!name.trim()) return;
    try {
      setError(null);
      await api("/projects", "POST", { name: name.trim(), instructions: "", sources: [] });
      setName("");
      await qc.invalidateQueries({ queryKey: ["/projects"] });
    } catch (cause) { setError(cause); }
  }
  async function update(value: object) {
    if (!project) return;
    try {
      setError(null);
      await api(`/projects/${project.id}`, "PATCH", value);
      await qc.invalidateQueries({ queryKey: ["/projects"] });
    } catch (cause) { setError(cause); }
  }
  return <main className="page projects-page">
    <Title title={t("プロジェクト")} sub={t("会話、指示、資料テキストをまとめます。既存の会話はそのまま残ります。")} />
    <ErrorBox error={error || projects.error || conversations.error} />
    <form onSubmit={create} className="folder-create"><input aria-label={t("プロジェクト名")} value={name} onChange={event => setName(event.target.value)} placeholder={t("新しいプロジェクト")} maxLength={100} /><Button type="submit">{t("作成")}</Button></form>
    <div className="folder-chips">{projects.data?.map(item => <Link key={item.id} to={`/projects/${item.id}`}>{item.data.name}</Link>)}</div>
    {project && <section className="project-detail">
      <h2>{project.data.name}</h2>
      <Link to={`/projects/${project.id}/chat`}>{t("このプロジェクトで新しいチャット")}</Link>
      <label>{t("プロジェクトの指示")}<textarea value={instructions} onChange={event => setInstructions(event.target.value)} maxLength={20000} /></label>
      <Button variant="outline" onClick={() => update({ instructions })}>{t("指示を保存")}</Button>
      <h3>{t("資料テキスト")}</h3>
      <p>{t("貼り付けた資料は、このプロジェクトの会話で参照されます。")}</p>
      <ul>{project.data.sources.map((item, index) => <li key={index}><span>{item.slice(0, 100)}</span><button type="button" onClick={() => update({ sources: project.data.sources.filter((_, i) => i !== index) })}>{t("削除")}</button></li>)}</ul>
      <textarea aria-label={t("資料テキスト")} value={source} onChange={event => setSource(event.target.value)} placeholder={t("参照したい資料を貼り付け")} maxLength={20000} />
      <Button variant="outline" disabled={!source.trim() || project.data.sources.length >= 20} onClick={() => { update({ sources: [...project.data.sources, source.trim()] }); setSource(""); }}>{t("資料を追加")}</Button>
      <h3>{t("会話")}</h3>
      {conversations.data?.filter(item => item.data.project_id === project.id).map(item => <p key={item.id}><Link to={`/chat/${item.id}`}>{item.data.title}</Link></p>)}
      <Button variant="outline" onClick={async () => { if (!confirm(t("プロジェクトを削除しますか？ 会話は残ります。"))) return; try { await api(`/projects/${project.id}`, "DELETE"); await qc.invalidateQueries({ queryKey: ["/projects"] }); await qc.invalidateQueries({ queryKey: ["/conversations"] }); location.href = "/projects"; } catch (cause) { setError(cause); } }}>{t("プロジェクトを削除")}</Button>
    </section>}
  </main>;
}
