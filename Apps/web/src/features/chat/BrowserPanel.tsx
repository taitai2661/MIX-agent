import { t } from "@/app/i18n";
import { api } from "@/app/api";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState, type MouseEvent } from "react";
import type { Run } from "./types";

type Frame = { artifact_id: string; url: string; tool: string; call_id?: string };

export function BrowserPanel({ runId, run, onChange }: { runId: string; run?: Run; onChange: () => void }) {
  const qc = useQueryClient();
  const { data: frames = [] } = useQuery<Frame[]>({
    queryKey: ["browser-frames", runId],
    queryFn: () => api(`/runs/${runId}/browser-frames`),
    refetchInterval: run && ["running", "waiting_approval", "paused"].includes(run.status) ? 1500 : false,
  });
  const [selected, setSelected] = useState<string | null>(null);
  const [typing, setTyping] = useState("");
  const [hideTyping, setHideTyping] = useState(true);
  const [expanded, setExpanded] = useState(false);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  if (!frames.length) return null;
  const frame = frames.find(item => item.artifact_id === selected) || frames.at(-1)!;
  const atLatest = frame.artifact_id === frames.at(-1)?.artifact_id;
  const manual = !!run?.browser_manual_active && ["paused", "waiting_approval"].includes(run.status);
  const image = `/api/v1/runs/${runId}/browser-frames/${frame.artifact_id}`;
  async function action(path: string, body?: unknown) {
    setWorking(true);
    setError("");
    try {
      await api(`/runs/${runId}/${path}`, "POST", body);
      setSelected(null);
      await qc.invalidateQueries({ queryKey: ["browser-frames", runId] });
      onChange();
    } catch (cause) {
      setError(cause instanceof Error ? t(cause.message) : t("操作できませんでした"));
    } finally {
      setWorking(false);
    }
  }
  function click(event: MouseEvent<HTMLImageElement>) {
    if (!manual || working || !atLatest) return;
    const target = event.currentTarget;
    const rect = target.getBoundingClientRect();
    if (!target.naturalWidth || !target.naturalHeight || !rect.width || !rect.height) return;
    // Map the pointer against the rendered image box, not the scroll container,
    // so clicks stay correct regardless of the surrounding layout.
    const x = (event.clientX - rect.left) * target.naturalWidth / rect.width;
    const y = (event.clientY - rect.top) * target.naturalHeight / rect.height;
    void action("browser-manual", { action: "click", x, y });
  }
  return <section className="browser-panel" aria-label={t("ブラウザ操作画面")}>
    <div className="browser-panel-heading"><strong>{t("ブラウザ画面")}</strong><span>{frames.length} {t("操作")}</span></div>
    <p className="browser-panel-url" title={frame.url}>{frame.url}</p>
    <div className="browser-panel-image"><img src={image} alt={`${frame.tool} 後のブラウザ画面`} onClick={click} /></div>
    <div className="browser-panel-actions">
      <button type="button" onClick={() => setExpanded(true)}>{t("拡大")}</button>
      {run?.status === "running" && <button type="button" disabled={working || run.browser_pause_requested} onClick={() => action("browser-pause")}>{run.browser_pause_requested ? t("停止待ち…") : t("一時停止して操作")}</button>}
      {run?.status === "waiting_approval" && !manual && run.approvals.some(item => item.status === "pending" && item.data.tool.startsWith("browser_")) && <button type="button" disabled={working} onClick={() => action("browser-pause")}>{t("承認前に手動操作")}</button>}
      {manual && <button type="button" disabled={working} onClick={() => action("browser-return")}>{t("AIに戻す")}</button>}
    </div>
    <div className="browser-panel-timeline" aria-label={t("画面履歴")}>{frames.map((item, index) =>
      <button type="button" key={item.artifact_id} className={item.artifact_id === frame.artifact_id ? "selected" : ""} onClick={() => setSelected(item.artifact_id)}>{index + 1}. {item.tool === "manual" ? t("手動操作") : item.tool.replace("browser_", "")}</button>
    )}</div>
    {manual && <div className="browser-panel-manual">
      <p>{t("最新の画面をクリックして操作できます。入力内容は操作履歴に保存しません。")}</p>
      {run?.status === "waiting_approval" && <p>{t("手動操作の終了後、待機中の操作を許可または拒否してください。")}</p>}
      <div><input type={hideTyping ? "password" : "text"} value={typing} disabled={!atLatest} onChange={event => setTyping(event.target.value)} placeholder={t("選択中の欄に入力")} aria-label={t("ブラウザへの入力")} autoComplete="off" /><label><input type="checkbox" checked={!hideTyping} onChange={event => setHideTyping(!event.target.checked)} />{t("表示")}</label><button type="button" disabled={working || !typing || !atLatest} onClick={async () => { const value = typing; setTyping(""); await action("browser-manual", { action: "type", text: value }); }}>{t("入力")}</button></div>
      <div>{["Enter", "Tab", "Backspace", "Escape"].map(key => <button type="button" key={key} disabled={working || !atLatest} onClick={() => action("browser-manual", { action: "key", key })}>{key}</button>)}<button type="button" disabled={working || !atLatest} onClick={() => action("browser-manual", { action: "scroll", dy: 500 })}>{t("下へ")}</button><button type="button" disabled={working || !atLatest} onClick={() => action("browser-manual", { action: "scroll", dy: -500 })}>{t("上へ")}</button></div>
    </div>}
    {error && <p role="alert">{error}</p>}
    {expanded && <div className="browser-panel-overlay" role="dialog" aria-modal="true" aria-label={t("ブラウザ画面の拡大表示")} onClick={() => setExpanded(false)}><button type="button" onClick={() => setExpanded(false)}>{t("閉じる")}</button><img src={image} alt={t("拡大したブラウザ画面")} onClick={event => event.stopPropagation()} /></div>}
  </section>;
}
