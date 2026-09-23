import { api } from "@/app/api";
import { Button } from "@/components/button";
import { ErrorBox, Field, Title, useRows } from "@/components/shared";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";

export function Auto() {
  const setting = useQuery({ queryKey: ["/settings"], queryFn: () => api("/settings") });
  const models = useRows("/models");
  const qc = useQueryClient();
  const [error, setError] = useState<unknown>(null);
  const [saved, setSaved] = useState(false);
  const [query, setQuery] = useState("");
  const [selectedOnly, setSelectedOnly] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (setting.data) setSelected(new Set(setting.data.data.auto_model_ids || []));
  }, [setting.data, setting.dataUpdatedAt]);

  const visible = useMemo(() => (models.data || []).filter((model) => {
    const name = model.data.name || model.data.model_id || "";
    return (!selectedOnly || selected.has(model.id)) && name.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase());
  }), [models.data, query, selected, selectedOnly]);

  const updateVisible = (checked: boolean) => setSelected((current) => {
    const next = new Set(current);
    visible.forEach((model) => checked ? next.add(model.id) : next.delete(model.id));
    return next;
  });

  return <>
    <Title title="Auto" sub="タスクに合わせてモデルを選択します。" />
    <ErrorBox error={error || setting.error} />
    {setting.data && <form className="card" key={setting.dataUpdatedAt} onSubmit={async (event) => {
      event.preventDefault();
      const form = new FormData(event.currentTarget);
      try {
        await api("/settings", "PUT", {
          auto_model_ids: [...selected],
          auto_retry_count: Number(form.get("auto_retry_count")),
          auto_dynamic_switching: form.get("auto_dynamic_switching") === "on",
        });
        setSaved(true);
        qc.invalidateQueries({ queryKey: ["/settings"] });
      } catch (cause) { setError(cause); }
    }}>
      <Field label="Autoで使用可能なモデル" hint="選択したモデルから、タスクやCapability、過去の実績に合わせて選びます。">
        <div className="auto-model-picker">
          <div className="auto-model-toolbar">
            <input aria-label="Auto候補を検索" className="auto-model-search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="モデルを検索" type="search" />
            <span className="auto-model-count">{selected.size}件を選択中</span>
          </div>
          <div className="auto-model-actions">
            <button type="button" className="btn-ghost" onClick={() => setSelectedOnly((value) => !value)} aria-pressed={selectedOnly}>{selectedOnly ? "すべて表示" : "選択済みのみ"}</button>
            <button type="button" className="btn-ghost" onClick={() => updateVisible(true)} disabled={!visible.length}>すべて選択</button>
            <button type="button" className="btn-ghost" onClick={() => updateVisible(false)} disabled={!visible.length}>すべて解除</button>
          </div>
          <div className="auto-model-list" aria-label="Autoで使用可能なモデル">
            {visible.map((model) => <label className="auto-model-option" key={model.id}>
              <input checked={selected.has(model.id)} onChange={() => setSelected((current) => {
                const next = new Set(current); next.has(model.id) ? next.delete(model.id) : next.add(model.id); return next;
              })} type="checkbox" />
              <span className="auto-model-name">{model.data.name || model.data.model_id}</span>
              <small>{model.data.context_window ? `${model.data.context_window.toLocaleString()} context` : "Context Window未設定"}</small>
            </label>)}
            {!visible.length && <p className="auto-model-empty">該当するモデルがありません。</p>}
          </div>
        </div>
      </Field>
      <Field label="実行中のモデル切替" hint="Tool結果や残りのタスクを見て、次のモデル呼び出し前に必要な場合だけ切り替えます。">
        <input name="auto_dynamic_switching" type="checkbox" defaultChecked={setting.data.data.auto_dynamic_switching !== false} />
      </Field>
      <Field label="Auto実行の再試行回数" hint="一時的なProvider障害時に、未使用の候補へ切り替える回数です。">
        <input name="auto_retry_count" type="number" min="0" step="1" defaultValue={setting.data.data.auto_retry_count ?? 3} />
      </Field>
      <Button>保存</Button>{saved && <span className="success">保存しました</span>}
    </form>}
  </>;
}
