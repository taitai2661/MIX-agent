import { api } from "@/app/api";
import { Button } from "@/components/button";
import { Check, ChevronLeft, ChevronRight, Compass, Globe2, PlugZap, Settings2, ShieldCheck, Sparkles, Wrench } from "lucide-react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";

import { ErrorBox } from "@/components/shared";
import { MCP } from "@/features/mcp/MCP";
import { Models } from "@/features/providers/Models";
import { Providers } from "@/features/providers/Providers";
import { General } from "@/features/settings/General";
import { Tools } from "@/features/tools/Tools";

const steps = [
  { name: "Browser", title: "ブラウザーを準備", description: "Webページを操作する機能を選びます。", icon: Globe2 },
  { name: "Provider", title: "AIを接続", description: "利用するProviderを登録します。モデルは接続後に取得できます。", icon: PlugZap },
  { name: "モデル", title: "モデルを確認", description: "取得したモデルと対応機能を確認します。", icon: Sparkles },
  { name: "一般・通信", title: "基本設定を整える", description: "既定モデルと外部サービスへの通信を設定します。", icon: Settings2 },
  { name: "Tools", title: "Toolsを選ぶ", description: "使いたい機能と権限を確認します。", icon: Wrench },
  { name: "MCP", title: "MCPを接続", description: "必要な外部ツールを追加します。後から設定することもできます。", icon: Compass },
];

export function Setup() {
  const [step, setStep] = useState(0),
    [error, setError] = useState<unknown>(null),
    [busy, setBusy] = useState(false),
    [installBrowser, setInstallBrowser] = useState(true);
  const navigate = useNavigate();
  const pages = [
    <section className="card setup-browser-card">
      <div className="setup-browser-icon"><Globe2 size={24} /></div>
      <div>
        <h2>Browser Tool</h2>
        <p>Playwright Chromiumを専用コンテナへ導入します。セットアップ完了後にバックグラウンドで始まるため、待たずに使い始められます。</p>
        <label className="setup-browser-choice">
          <input type="checkbox" checked={installBrowser} onChange={(event) => setInstallBrowser(event.target.checked)} />
          <span><strong>Browserを導入する</strong><small>Webページの閲覧と操作に使用します</small></span>
        </label>
        <p className="muted">数百MBのダウンロードが必要です。後から設定画面で導入・再試行できます。</p>
      </div>
    </section>,
    <Providers />, <Models />, <General />, <Tools />, <MCP />,
  ];

  async function complete() {
    setBusy(true);
    setError(null);
    try {
      const s = await api("/settings");
      const { default_model_id = "", auto_model_ids = [], allowed_domains = [] } = s.data;
      await api("/settings", "PUT", {
        default_model_id,
        auto_model_ids,
        allowed_domains,
        setup_complete: true,
        browser_install_requested: installBrowser,
      });
      if (installBrowser) {
        await api("/browser/enable", "POST");
        await api("/browser/install", "POST");
      }
      navigate("/");
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }

  return (
    <main className="setup-page">
      <div className="setup-shell">
        <aside className="setup-sidebar" aria-label="セットアップの手順">
          <div className="setup-sidebar-heading">
            <span className="setup-mark"><Sparkles size={19} /></span>
            <div><strong>MIX agent</strong><span>初期セットアップ</span></div>
          </div>
          <div className="setup-progress-copy"><span>セットアップの進行状況</span><strong>{step + 1} / {steps.length}</strong></div>
          <div className="setup-progress-track"><span style={{ width: `${((step + 1) / steps.length) * 100}%` }} /></div>
          <nav className="setup-steps">
            {steps.map(({ name, icon: Icon }, i) => (
              <button key={name} type="button" className={step === i ? "selected" : ""} aria-current={step === i ? "step" : undefined} onClick={() => { setStep(i); setError(null); }}>
                <span className="setup-step-icon">{i < step ? <Check size={16} /> : <Icon size={17} />}</span>
                <span className="setup-step-label"><small>STEP {String(i + 1).padStart(2, "0")}</small><strong>{name}</strong></span>
                {step === i && <ChevronRight size={16} className="setup-step-arrow" />}
              </button>
            ))}
          </nav>
          <div className="setup-sidebar-note"><ShieldCheck size={18} /><span>設定はセットアップ後もいつでも変更できます。</span></div>
        </aside>
        <div className="setup-main">
          <header className="setup-header">
            <p className="eyebrow">GETTING STARTED · STEP {String(step + 1).padStart(2, "0")}</p>
            <h1>{steps[step].title}</h1>
            <p>{steps[step].description}</p>
          </header>
          <div className="setup-content" key={step}>{pages[step]}</div>
          <ErrorBox error={error} />
          <div className="setup-footer">
            <Button variant="ghost" onClick={() => step === 0 ? navigate("/") : setStep(step - 1)}>
              {step === 0 ? "あとで設定する" : <><ChevronLeft size={15} />戻る</>}
            </Button>
            {step < steps.length - 1 ? (
              <Button onClick={() => { setStep(step + 1); setError(null); }}>次へ<ChevronRight size={15} /></Button>
            ) : (
              <Button disabled={busy} onClick={complete}>{busy ? "処理中…" : "設定を完了"}<Check size={15} /></Button>
            )}
          </div>
        </div>
      </div>
    </main>
  );
}
