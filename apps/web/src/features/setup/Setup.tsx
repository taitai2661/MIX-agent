import { api } from "@/app/api";
import { Button } from "@/components/button";
import { Check, ChevronRight } from "lucide-react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";

import { ErrorBox } from "@/components/shared";
import { MCP } from "@/features/mcp/MCP";
import { Models } from "@/features/providers/Models";
import { Providers } from "@/features/providers/Providers";
import { General } from "@/features/settings/General";
import { Tools } from "@/features/tools/Tools";

export function Setup() {
  const [step, setStep] = useState(0),
    [error, setError] = useState<unknown>(null),
    [installBrowser, setInstallBrowser] = useState(true);
  const navigate = useNavigate();
  const pages = [
    <section className="card"><h2>Browser Tool</h2><p>Playwright Chromiumを専用コンテナへ導入します。導入はセットアップ完了後にバックグラウンドで始まり、ほかの設定は待たずに使い始められます。</p><label className="check"><input type="checkbox" checked={installBrowser} onChange={(event) => setInstallBrowser(event.target.checked)} /> Browser（Playwright Chromium）を導入する</label><p className="muted">数百MBのダウンロードが必要です。後から設定画面で導入・再試行できます。</p></section>,
    <Providers />, <Models />, <General />, <Tools />, <MCP />,
  ];
  return (
    <main className="setup-page">
      <div className="setup-header">
        <p className="eyebrow">LET’S MAKE IT YOURS</p>
        <h1>ワークスペースの準備</h1>
        <p>
          Providerを接続すると、モデル一覧とAuto候補を自動設定します。権限はあとから調整できます。
        </p>
        <div className="setup-steps">
          {["Browser", "Provider", "モデル", "既定・通信", "Tools", "MCP"].map(
            (name, i) => (
              <button
                key={name}
                className={step === i ? "selected" : ""}
                onClick={() => setStep(i)}
              >
                <span>{i < step ? <Check size={13} /> : i + 1}</span>
                {name}
              </button>
            ),
          )}
        </div>
      </div>
      {pages[step]}
      <ErrorBox error={error} />
      <div className="setup-footer">
        <Button variant="ghost" onClick={() => navigate("/")}>
          あとで設定する
        </Button>
        {step < 5 ? (
          <Button onClick={() => setStep(step + 1)}>
            次へ
            <ChevronRight size={15} />
          </Button>
        ) : (
          <Button
            onClick={async () => {
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
              }
            }}
          >
            設定を完了
            <Check size={15} />
          </Button>
        )}
      </div>
    </main>
  );
}
