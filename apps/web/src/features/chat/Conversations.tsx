import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Title } from "@/components/shared";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Archive, Download, Folder, Pin, RotateCcw, Search, Trash2 } from "lucide-react";
import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";

type FolderRow = Row & { data: { name: string } };
type ConversationRow = Row & { data: { title: string; folder_id?: string | null; pinned?: boolean } };
const states: Record<string, [string, string]> = { active: ["会話を管理", "検索、整理、アーカイブ、共有を行えます。"], archived: ["アーカイブ", "保管した会話です。"], trash: ["ごみ箱", "30日後に関連データと添付を含めて完全に削除されます。"] };

export function Conversations() {
  const { state = "active" } = useParams(); const current = states[state] ? state : "active";
  const [q, setQ] = useState(""); const [folderName, setFolderName] = useState("");
  const qc = useQueryClient(); const navigate = useNavigate();
  const folders = useQuery<FolderRow[]>({ queryKey: ["/conversation-folders"], queryFn: () => api("/conversation-folders") });
  const conversations = useQuery<ConversationRow[]>({ queryKey: ["/conversations", current, q], queryFn: () => api(`/conversations?state=${current}&q=${encodeURIComponent(q)}`) });
  const refresh = () => Promise.all([qc.invalidateQueries({ queryKey: ["/conversations"] }), qc.invalidateQueries({ queryKey: ["/conversation-folders"] })]);
  const change = async (id: string, data: object) => { await api(`/conversations/${id}/state`, "PATCH", data); await refresh(); };
  const remove = async (id: string, permanent = false) => { if (!confirm(permanent ? "この会話を完全に削除します。元に戻せません。" : "会話をごみ箱へ移動しますか？")) return; await api(`/conversations/${id}?permanent=${permanent}`, "DELETE"); await refresh(); };
  const download = async (id: string, title: string) => { const r = await fetch(`/api/v1/conversations/${id}/markdown`); if (!r.ok) throw new Error("書き出しに失敗しました"); const a = document.createElement("a"); a.href = URL.createObjectURL(await r.blob()); a.download = `${title}.md`; a.click(); URL.revokeObjectURL(a.href); };
  return <main className="page conversation-manager">
    <Title title={states[current][0]} sub={states[current][1]} />
    {current === "active" && <><form className="folder-create" onSubmit={async e => { e.preventDefault(); if (!folderName.trim()) return; await api("/conversation-folders", "POST", { name: folderName.trim() }); setFolderName(""); await refresh(); }}><Folder size={16} /><input value={folderName} onChange={e => setFolderName(e.target.value)} placeholder="新しいフォルダ" /><Button type="submit" variant="outline">作成</Button></form><div className="folder-chips">{folders.data?.map(f => <span key={f.id}><Folder size={13} />{f.data.name}<button onClick={async () => { if (confirm("フォルダを削除しますか？ 会話は未分類に戻ります。")) { await api(`/conversation-folders/${f.id}`, "DELETE"); await refresh(); } }}>×</button></span>)}</div></>}
    <label className="conversation-search"><Search size={16} /><input value={q} onChange={e => setQ(e.target.value)} placeholder="タイトルと会話内容を検索" /></label>
    {conversations.error && <ErrorBox error={conversations.error} />}<div className="conversation-list">{conversations.data?.map(row => <article key={row.id} className="conversation-row"><div><Link to={`/chat/${row.id}`}><b>{row.data.title}</b></Link><small>{row.data.pinned ? "ピン留め · " : ""}{folders.data?.find(f => f.id === row.data.folder_id)?.data.name || "未分類"}</small></div><div className="row-actions">{current === "active" && <><button className="icon" onClick={() => change(row.id, { pinned: !row.data.pinned })}><Pin size={16} /></button><select value={row.data.folder_id || ""} onChange={e => change(row.id, { folder_id: e.target.value || null })}><option value="">未分類</option>{folders.data?.map(f => <option key={f.id} value={f.id}>{f.data.name}</option>)}</select><button className="icon" onClick={() => change(row.id, { archived: true })}><Archive size={16} /></button><button className="icon" onClick={() => download(row.id, row.data.title)}><Download size={16} /></button></>}{current === "archived" && <button className="icon" onClick={() => change(row.id, { archived: false })}><RotateCcw size={16} /></button>}{current === "trash" && <><button className="icon" onClick={async () => { await api(`/conversations/${row.id}/restore`, "POST"); await refresh(); }}><RotateCcw size={16} /></button><button className="icon danger" onClick={() => remove(row.id, true)}><Trash2 size={16} /></button></>}{current !== "trash" && <button className="icon danger" onClick={() => remove(row.id)}><Trash2 size={16} /></button>}</div></article>)}{!conversations.data?.length && <p className="empty">該当する会話はありません。</p>}</div>
    <nav className="conversation-links"><button onClick={() => navigate("/history")}>会話</button><button onClick={() => navigate("/history/archived")}>アーカイブ</button><button onClick={() => navigate("/history/trash")}>ごみ箱</button></nav>
  </main>;
}
