import type { Row } from "@/app/api";
import { Check, ChevronDown, Search, Sparkles } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";

export function ModelPicker({ models, value, onChange, temporaryMode, allowTools, onTemporaryModeChange, onAllowToolsChange }: { models?: Row[]; value: string; onChange: (id: string) => void; temporaryMode: boolean; allowTools: boolean; onTemporaryModeChange: (value: boolean) => void; onAllowToolsChange: (value: boolean) => void }) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const navigate = useNavigate();
  const choices = useMemo(() => [{ id: "auto", created_at: "", data: { name: "Auto", model_id: "リクエストごとに最適なモデルを選択" } } as Row, ...(models || [])], [models]);
  const selected = choices.find((item) => item.id === value);
  const filtered = useMemo(() => choices.filter((item) =>
    String(item.data.name || item.data.model_id).toLocaleLowerCase().includes(query.toLocaleLowerCase()),
  ), [choices, query]);

  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    requestAnimationFrame(() => input.current?.focus());
    return () => document.removeEventListener("mousedown", close);
  }, [open]);
  useEffect(() => setActive(0), [query, open]);

  function choose(item: Row) {
    onChange(item.id);
    setOpen(false);
    setQuery("");
  }
  function keyboard(event: React.KeyboardEvent) {
    if (event.key === "Escape") return setOpen(false);
    if (event.key === "ArrowDown") {
      event.preventDefault(); setActive((index) => Math.min(index + 1, filtered.length - 1));
    }
    if (event.key === "ArrowUp") {
      event.preventDefault(); setActive((index) => Math.max(index - 1, 0));
    }
    if (event.key === "Enter" && filtered[active]) {
      event.preventDefault(); choose(filtered[active]);
    }
  }
  return (
    <div className="model-picker" ref={root} onKeyDown={keyboard}>
      <button className="model-trigger" aria-expanded={open} aria-haspopup="listbox" onClick={() => setOpen((shown) => !shown)} type="button">
        <Sparkles size={16} />
        <span>{selected?.data.name || selected?.data.model_id || "モデルを選択"}</span>
        <ChevronDown size={16} />
      </button>
      {open && <div className="model-menu" role="dialog" aria-label="モデルを選択">
        <label className="model-search"><Search size={15} /><input ref={input} value={query} onChange={(event) => setQuery(event.target.value)} placeholder="モデルを検索" /></label>
        <div className="model-options" role="listbox">
          {filtered.map((item, index) => <button key={item.id} className={index === active ? "active" : ""} aria-selected={value === item.id} onMouseEnter={() => setActive(index)} onClick={() => choose(item)} role="option" type="button">
            <span className="model-option-icon"><Sparkles size={14} /></span>
            <span><b>{item.data.name || item.data.model_id}</b><small>{item.data.model_id}</small></span>
            {value === item.id && <Check size={16} />}
          </button>)}
          {!filtered.length && <div className="model-empty"><p>利用できるモデルがありません</p><button type="button" onClick={() => navigate("/settings/models")}>モデルを設定</button></div>}
        </div>
        <div className="temporary-settings">
          <label><input type="checkbox" checked={temporaryMode} onChange={(event) => onTemporaryModeChange(event.target.checked)} /> 一時モード</label>
          {temporaryMode && <>
            <small>Memoryを使わず、この会話と添付を保存しません</small>
            <label><input type="checkbox" checked={allowTools} onChange={(event) => onAllowToolsChange(event.target.checked)} /> Toolを許可</label>
            {allowTools && <small>Toolの外部送信や副作用は残る場合があります</small>}
          </>}
        </div>
      </div>}
    </div>
  );
}
