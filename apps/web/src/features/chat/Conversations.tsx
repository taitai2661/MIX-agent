import { t } from "@/app/i18n";
import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { Empty, ErrorBox, Title } from "@/components/shared";
import { Skeleton } from "@/components/loading";
import { ConfirmModal } from "@/components/modal";
import {
  CONVERSATION_PAGE_SIZE,
  nextConversationOffset,
} from "@/lib/conversations";
import {
  HISTORY_GROUPS,
  historyGroup,
  historyTime,
  type HistoryGroup,
} from "@/lib/format";
import {
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import {
  Archive,
  Download,
  Folder,
  Pin,
  RotateCcw,
  Search,
  Trash2,
} from "lucide-react";
import { Fragment, useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";

type FolderRow = Row & { data: { name: string } };
type ConversationRow = Row & {
  data: {
    title: string;
    folder_id?: string | null;
    project_id?: string | null;
    pinned?: boolean;
    last_message_at?: string | null;
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
  const conversations = useInfiniteQuery({
    queryKey: ["/conversations", current, searchQ],
    initialPageParam: 0,
    queryFn: ({ pageParam, signal }) =>
      api<ConversationRow[]>(
        `/conversations?state=${current}&q=${encodeURIComponent(searchQ)}&limit=${CONVERSATION_PAGE_SIZE}&offset=${pageParam}`,
        "GET",
        undefined,
        undefined,
        { signal },
      ),
    getNextPageParam: (lastPage, _pages, lastPageParam) =>
      nextConversationOffset(lastPage.length, lastPageParam),
  });
  const conversationRows = useMemo(
    () => conversations.data?.pages.flatMap((page) => page) ?? [],
    [conversations.data],
  );
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
  const grouped = useMemo(() => {
    const groups: Record<HistoryGroup, ConversationRow[]> = {
      today: [],
      yesterday: [],
      week: [],
      older: [],
    };
    for (const row of conversationRows) {
      groups[historyGroup(row.data.last_message_at || row.created_at)].push(
        row,
      );
    }
    return groups;
  }, [conversationRows]);
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
                  aria-label={t("{name}フォルダを削除", { name: f.data.name })}
                  onClick={() =>
                    setPending({
                      title: t("フォルダを削除"),
                      message: t(
                        "フォルダを削除しますか？ 会話は未分類に戻ります。",
                      ),
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
          {HISTORY_GROUPS.map((group) => {
            const rows = grouped[group];
            if (!rows.length) return null;
            const label = {
              today: t("今日"),
              yesterday: t("昨日"),
              week: t("過去7日"),
              older: t("それ以前"),
            }[group];
            return (
              <Fragment key={group}>
                <h3 className="conversation-group-label">{label}</h3>
                {rows.map((row) => (
                  <article key={row.id} className="conversation-row">
                    <div>
                      <Link to={`/chat/${row.id}`}>
                        <b>{row.data.title}</b>
                      </Link>
                      <small>
                        {row.data.pinned ? t("ピン留め · ") : ""}
                        {folders.data?.find((f) => f.id === row.data.folder_id)
                          ?.data.name || t("未分類")}
                        {row.data.last_message_at
                          ? ` · ${historyTime(row.data.last_message_at)}`
                          : ""}
                      </small>
                    </div>
                    <div className="row-actions">
                      {current === "active" && (
                        <>
                          <button
                            className="icon"
                            aria-label={t(
                              row.data.pinned
                                ? "{title}のピン留めを解除"
                                : "{title}をピン留め",
                              { title: row.data.title },
                            )}
                            onClick={() =>
                              change(row.id, { pinned: !row.data.pinned })
                            }
                          >
                            <Pin size={16} />
                          </button>
                          <select
                            aria-label={t("{title}のフォルダ", {
                              title: row.data.title,
                            })}
                            value={row.data.folder_id || ""}
                            onChange={(e) =>
                              change(row.id, {
                                folder_id: e.target.value || null,
                              })
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
                            aria-label={t("{title}のプロジェクト", {
                              title: row.data.title,
                            })}
                            value={row.data.project_id || ""}
                            onChange={(e) =>
                              change(row.id, {
                                project_id: e.target.value || null,
                              })
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
                            aria-label={t("{title}をアーカイブ", {
                              title: row.data.title,
                            })}
                            onClick={() => change(row.id, { archived: true })}
                          >
                            <Archive size={16} />
                          </button>
                          <button
                            className="icon"
                            aria-label={t("{title}を書き出し", {
                              title: row.data.title,
                            })}
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
                            aria-label={t("{title}をアーカイブから戻す", {
                              title: row.data.title,
                            })}
                            onClick={() => change(row.id, { archived: false })}
                          >
                            <RotateCcw size={16} />
                          </button>
                          <button
                            className="icon"
                            aria-label={t("{title}を書き出し", {
                              title: row.data.title,
                            })}
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
                            aria-label={t("{title}を復元", {
                              title: row.data.title,
                            })}
                            onClick={async () => {
                              try {
                                setError(null);
                                await api(
                                  `/conversations/${row.id}/restore`,
                                  "POST",
                                );
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
                            aria-label={t("{title}を完全削除", {
                              title: row.data.title,
                            })}
                            onClick={() => remove(row.id, true)}
                          >
                            <Trash2 size={16} />
                          </button>
                        </>
                      )}
                      {current !== "trash" && (
                        <button
                          className="icon danger"
                          aria-label={t("{title}をごみ箱へ移動", {
                            title: row.data.title,
                          })}
                          onClick={() => remove(row.id)}
                        >
                          <Trash2 size={16} />
                        </button>
                      )}
                    </div>
                  </article>
                ))}
              </Fragment>
            );
          })}
          {!conversations.isError && !error && !conversationRows.length && (
            <Empty>{t("該当する会話はありません。")}</Empty>
          )}
          {conversations.hasNextPage && (
            <div className="conversation-list-more">
              <Button
                variant="outline"
                disabled={conversations.isFetchingNextPage}
                onClick={() => void conversations.fetchNextPage()}
              >
                {conversations.isFetchingNextPage
                  ? t("読み込み中…")
                  : t("以前の会話を読み込む")}
              </Button>
            </div>
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
