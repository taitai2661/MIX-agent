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
    [error, setError] = useState<unknown>(null);
  const navigate = useNavigate();
  const pages = [<Providers />, <Models />, <General />, <Tools />, <MCP />];
  return (
    <main className="setup-page">
      <div className="setup-header">
        <p className="eyebrow">LET’S MAKE IT YOURS</p>
        <h1>ワークスペースの準備</h1>
        <p>
          Providerを接続すると、モデル一覧とAuto候補を自動設定します。権限はあとから調整できます。
        </p>
        <div className="setup-steps">
          {["Provider", "モデル", "既定・通信", "Tools", "MCP"].map(
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
        {step < 4 ? (
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
                });
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
