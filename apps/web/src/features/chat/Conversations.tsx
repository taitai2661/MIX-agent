import { t } from "@/app/i18n";
import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { Empty, ErrorBox, Title } from "@/components/shared";
import { Skeleton } from "@/components/loading";
import { ConfirmModal } from "@/components/modal";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Archive,
  Download,
  Folder,
  Pin,
  RotateCcw,
  Search,
  Trash2,
} from "lucide-react";
import { useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";

type FolderRow = Row & { data: { name: string } };
type ConversationRow = Row & {
  data: {
    title: string;
    folder_id?: string | null;
    project_id?: string | null;
    pinned?: boolean;
  };
};
const states: Record<string, [string, string]> = {
  active: ["会話を管理", "検索、整理、アーカイブ、共有を行えます。"],
  archived: ["アーカイブ", "保管した会話です。"],
  trash: ["ごみ箱", "30日後に関連データと添付を含めて完全に削除されます。"],
};

type PendingAction = {
  title: string;
  message: string;
  confirmLabel: string;
  run: () => Promise<void>;
};

export function Conversations() {
  const { state = "active" } = useParams();
  const current = states[state] ? state : "active";
  const [q, setQ] = useState("");
  const [searchQ, setSearchQ] = useState("");
  const [folderName, setFolderName] = useState("");
  const [exportScope, setExportScope] = useState<
    "answers" | "conversation" | "activity"
  >("conversation");
  const [error, setError] = useState<unknown>(null);
  const [pending, setPending] = useState<PendingAction | null>(null);
  const qc = useQueryClient();
  const navigate = useNavigate();

  // One request per pause, not one per keystroke.
  useEffect(() => {
    const timer = setTimeout(() => setSearchQ(q), 250);
    return () => clearTimeout(timer);
  }, [q]);

  const folders = useQuery<FolderRow[]>({
    queryKey: ["/conversation-folders"],
    queryFn: () => api("/conversation-folders"),
  });
  const projects = useQuery<FolderRow[]>({
    queryKey: ["/projects"],
    queryFn: () => api("/projects"),
  });
  const conversations = useQuery<ConversationRow[]>({
    queryKey: ["/conversations", current, searchQ],
    queryFn: () =>
      api(`/conversations?state=${current}&q=${encodeURIComponent(searchQ)}`),
  });
  const refresh = () =>
    Promise.all([
      qc.invalidateQueries({ queryKey: ["/conversations"] }),
      qc.invalidateQueries({ queryKey: ["/conversation-folders"] }),
    ]);
  const change = async (id: string, data: object) => {
    setError(null);
    try {
      await api(`/conversations/${id}/state`, "PATCH", data);
      await refresh();
    } catch (cause) {
      setError(cause);
    }
  };
  const remove = (id: string, permanent = false) => {
    setPending({
      title: permanent ? t("会話を完全に削除") : t("ごみ箱へ移動"),
      message: t(
        permanent
          ? "この会話を完全に削除します。元に戻せません。"
          : "会話をごみ箱へ移動しますか？",
      ),
      confirmLabel: permanent ? t("完全に削除") : t("ごみ箱へ移動"),
      run: async () => {
        await api(`/conversations/${id}?permanent=${permanent}`, "DELETE");
        await refresh();
      },
    });
  };
  const download = async (id: string, title: string) => {
    try {
      setError(null);
      const r = await fetch(
        `/api/v1/conversations/${id}/markdown?scope=${exportScope}`,
        { credentials: "same-origin" },
      );
      if (!r.ok) throw new Error(t("書き出しに失敗しました"));
      const url = URL.createObjectURL(await r.blob());
      const a = document.createElement("a");
      a.href = url;
      a.download = `${title.replace(/[\\/:*?"<>|]/g, "_")}.md`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    } catch (cause) {
      setError(cause);
    }
  };

  return (
    <section className="page conversation-manager">
      <Title title={t(states[current][0])} sub={t(states[current][1])} />
      {current === "active" && (
        <>
          <form
            className="folder-create"
            onSubmit={async (e) => {
              e.preventDefault();
              if (!folderName.trim()) return;
              setError(null);
              try {
                await api("/conversation-folders", "POST", {
                  name: folderName.trim(),
                });
                setFolderName("");
                await refresh();
              } catch (cause) {
                setError(cause);
              }
            }}
          >
            <Folder size={16} />
            <input
              aria-label={t("新しいフォルダ名")}
              value={folderName}
              onChange={(e) => setFolderName(e.target.value)}
              placeholder={t("新しいフォルダ")}
            />
            <Button type="submit" variant="outline">
              {t("作成")}
            </Button>
          </form>
          <div className="folder-chips">
            {folders.data?.map((f) => (
              <span key={f.id}>
                <Folder size={13} />
                {f.data.name}
                <button
                  aria-label={`${f.data.name}フォルダを削除`}
                  onClick={() =>
                    setPending({
                      title: t("フォルダを削除"),
                      message:
                        "フォルダを削除しますか？ 会話は未分類に戻ります。",
                      confirmLabel: t("削除"),
                      run: async () => {
                        await api(`/conversation-folders/${f.id}`, "DELETE");
                        await refresh();
                      },
                    })
                  }
                >
                  ×
                </button>
              </span>
            ))}
          </div>
        </>
      )}
      <label className="conversation-search">
        <Search size={16} />
        <input
          aria-label={t("会話を検索")}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder={t("タイトルと会話内容を検索")}
        />
      </label>
      {current !== "trash" && (
        <label className="export-scope">
          {t("書き出し範囲")}{" "}
          <select
            value={exportScope}
            onChange={(e) =>
              setExportScope(e.target.value as typeof exportScope)
            }
          >
            <option value="answers">{t("回答のみ")}</option>
            <option value="conversation">{t("会話と添付名")}</option>
            <option value="activity">{t("会話とTool実行概要")}</option>
          </select>
        </label>
      )}
      <ErrorBox
        error={conversations.error || error}
        onRetry={() => {
          setError(null);
          void conversations.refetch();
        }}
      />
      {conversations.isLoading ? (
        <div className="conversation-list" aria-busy="true">
          {[0, 1, 2, 3].map((i) => (
            <div className="conversation-row" key={i}>
              <div style={{ width: "100%" }}>
                <Skeleton className="skeleton-title" />
                <Skeleton className="skeleton-text short" />
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div className="conversation-list">
          {conversations.data?.map((row) => (
            <article key={row.id} className="conversation-row">
              <div>
                <Link to={`/chat/${row.id}`}>
                  <b>{row.data.title}</b>
                </Link>
                <small>
                  {row.data.pinned ? t("ピン留め · ") : ""}
                  {folders.data?.find((f) => f.id === row.data.folder_id)?.data
                    .name || t("未分類")}
                </small>
              </div>
              <div className="row-actions">
                {current === "active" && (
                  <>
                    <button
                      className="icon"
                      aria-label={`${row.data.title}を${row.data.pinned ? t("ピン留め解除") : t("ピン留め")}`}
                      onClick={() =>
                        change(row.id, { pinned: !row.data.pinned })
                      }
                    >
                      <Pin size={16} />
                    </button>
                    <select
                      aria-label={`${row.data.title}のフォルダ`}
                      value={row.data.folder_id || ""}
                      onChange={(e) =>
                        change(row.id, { folder_id: e.target.value || null })
                      }
                    >
                      <option value="">{t("未分類")}</option>
                      {folders.data?.map((f) => (
                        <option key={f.id} value={f.id}>
                          {f.data.name}
                        </option>
                      ))}
                    </select>
                    <select
                      aria-label={`${row.data.title}のプロジェクト`}
                      value={row.data.project_id || ""}
                      onChange={(e) =>
                        change(row.id, { project_id: e.target.value || null })
                      }
                    >
                      <option value="">{t("プロジェクトなし")}</option>
                      {projects.data?.map((p) => (
                        <option key={p.id} value={p.id}>
                          {p.data.name}
                        </option>
                      ))}
                    </select>
                    <button
                      className="icon"
                      aria-label={`${row.data.title}をアーカイブ`}
                      onClick={() => change(row.id, { archived: true })}
                    >
                      <Archive size={16} />
                    </button>
                    <button
                      className="icon"
                      aria-label={`${row.data.title}を書き出し`}
                      onClick={() => download(row.id, row.data.title)}
                    >
                      <Download size={16} />
                    </button>
                  </>
                )}
                {current === "archived" && (
                  <>
                    <button
                      className="icon"
                      aria-label={`${row.data.title}をアーカイブから戻す`}
                      onClick={() => change(row.id, { archived: false })}
                    >
                      <RotateCcw size={16} />
                    </button>
                    <button
                      className="icon"
                      aria-label={`${row.data.title}を書き出し`}
                      onClick={() => download(row.id, row.data.title)}
                    >
                      <Download size={16} />
                    </button>
                  </>
                )}
                {current === "trash" && (
                  <>
                    <button
                      className="icon"
                      aria-label={`${row.data.title}を復元`}
                      onClick={async () => {
                        try {
                          setError(null);
                          await api(`/conversations/${row.id}/restore`, "POST");
                          await refresh();
                        } catch (cause) {
                          setError(cause);
                        }
                      }}
                    >
                      <RotateCcw size={16} />
                    </button>
                    <button
                      className="icon danger"
                      aria-label={`${row.data.title}を完全削除`}
                      onClick={() => remove(row.id, true)}
                    >
                      <Trash2 size={16} />
                    </button>
                  </>
                )}
                {current !== "trash" && (
                  <button
                    className="icon danger"
                    aria-label={`${row.data.title}をごみ箱へ移動`}
                    onClick={() => remove(row.id)}
                  >
                    <Trash2 size={16} />
                  </button>
                )}
              </div>
            </article>
          ))}
          {!conversations.isError && !error && !conversations.data?.length && (
            <Empty>{t("該当する会話はありません。")}</Empty>
          )}
        </div>
      )}
      <nav className="conversation-links">
        <button
          aria-current={current === "active" ? "page" : undefined}
          onClick={() => navigate("/history")}
        >
          {t("会話")}
        </button>
        <button
          aria-current={current === "archived" ? "page" : undefined}
          onClick={() => navigate("/history/archived")}
        >
          {t("アーカイブ")}
        </button>
        <button
          aria-current={current === "trash" ? "page" : undefined}
          onClick={() => navigate("/history/trash")}
        >
          {t("ごみ箱")}
        </button>
      </nav>
      {pending && (
        <ConfirmModal
          title={pending.title}
          message={pending.message}
          confirmLabel={pending.confirmLabel}
          tone="danger"
          onConfirm={() => {
            const action = pending.run;
            setPending(null);
            void action().catch(setError);
          }}
          onCancel={() => setPending(null)}
        />
      )}
    </section>
  );
}
