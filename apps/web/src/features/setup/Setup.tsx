import { api, type Row } from "@/app/api";
import { Button } from "@/components/button";
import { useQueryClient } from "@tanstack/react-query";
import {
  Check,
  ChevronLeft,
  ChevronRight,
  Globe2,
  Sparkles,
  Bot,
  Zap,
  Shield,
} from "lucide-react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ErrorBox, Field } from "@/components/shared";
import { getLanguage, setLanguage, t, useLanguage } from "@/app/i18n";

export function Setup() {
  useLanguage();
  const [step, setStep] = useState(0),
    [error, setError] = useState<unknown>(null),
    [busy, setBusy] = useState(false);

  const [installBrowser, setInstallBrowser] = useState(true);
  const [selectedProvider, setSelectedProvider] = useState<
    "openai" | "anthropic" | "gemini" | ""
  >("");
  const [apiKey, setApiKey] = useState("");

  const navigate = useNavigate();
  const queryClient = useQueryClient();

  async function changeLanguage(value: "ja" | "en") {
    setLanguage(value);
    await api("/settings", "PUT", { ui_language: value });
  }

  async function connectProvider() {
    if (!selectedProvider || !apiKey) return true;
    setBusy(true);
    setError(null);
    try {
      const presets = {
        openai: {
          name: "OpenAI",
          kind: "openai",
          url: "https://api.openai.com/v1",
        },
        anthropic: {
          name: "Anthropic",
          kind: "anthropic",
          url: "https://api.anthropic.com",
        },
        gemini: {
          name: "Google Gemini",
          kind: "gemini",
          url: "https://generativelanguage.googleapis.com",
        },
      };
      const preset = presets[selectedProvider];

      await api("/providers", "POST", {
        name: preset.name,
        preset_id: selectedProvider,
        base_url: preset.url,
        api_key: apiKey,
        kind: preset.kind,
        allow_private: false,
        rate_limit_rpm: 0,
        rate_limit_period: "minute",
        extra_config: {},
      });
      return true;
    } catch (e) {
      setError(e);
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function complete() {
    setBusy(true);
    setError(null);
    try {
      if (selectedProvider && apiKey) {
        const ok = await connectProvider();
        if (!ok) return;
      }

      const s = await api<Row>("/settings");
      const {
        default_model_id = "",
        auto_model_ids = [],
        allowed_domains = [],
      } = s.data;
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
        // The install runs in the background after this screen closes, so the
        // global notifier needs the fresh status to show its start popup.
        await queryClient.invalidateQueries({ queryKey: ["/settings"] });
      }
      void navigate("/");
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }

  const handleNext = async () => {
    if (step === 1 && selectedProvider && apiKey) {
      const ok = await connectProvider();
      if (!ok) return;
    }
    setError(null);
    setStep(step + 1);
  };

  const steps = [
    {
      title: t("Welcome to MIX agent"),
      desc: t(
        "あなたの指示に従ってコードを書いたり、Webを調べたりする自律型AIエージェントです。設定はたったの数ステップで終わります。",
      ),
      content: (
        <div style={{ textAlign: "center", padding: "40px 0" }}>
          <Bot size={64} style={{ color: "var(--accent)", marginBottom: 20 }} />
          <h2>{t("AIエージェントを始めましょう")}</h2>
          <p style={{ color: "var(--muted)", marginTop: 10 }}>
            {t(
              "初期設定はすぐに完了します。後から設定画面でいつでも変更可能です。",
            )}
          </p>
          <div
            style={{
              display: "flex",
              justifyContent: "center",
              gap: 24,
              marginTop: 30,
            }}
          >
            <div style={{ textAlign: "center" }}>
              <Zap
                size={28}
                style={{ color: "var(--accent)", marginBottom: 8 }}
              />
              <p style={{ fontSize: 12, color: "var(--muted)" }}>
                {t("高速な応答")}
              </p>
            </div>
            <div style={{ textAlign: "center" }}>
              <Shield
                size={28}
                style={{ color: "var(--accent)", marginBottom: 8 }}
              />
              <p style={{ fontSize: 12, color: "var(--muted)" }}>
                {t("安全な実行環境")}
              </p>
            </div>
            <div style={{ textAlign: "center" }}>
              <Globe2
                size={28}
                style={{ color: "var(--accent)", marginBottom: 8 }}
              />
              <p style={{ fontSize: 12, color: "var(--muted)" }}>
                {t("Webアクセス")}
              </p>
            </div>
          </div>
        </div>
      ),
    },
    {
      title: t("AIモデルの接続"),
      desc: t("利用するAIプロバイダを選択し、APIキーを入力してください。"),
      content: (
        <div className="card form-grid" style={{ marginTop: 20 }}>
          <div className="setup-provider-grid">
            <Button
              variant={selectedProvider === "openai" ? "default" : "outline"}
              onClick={() => setSelectedProvider("openai")}
            >
              OpenAI
            </Button>
            <Button
              variant={selectedProvider === "anthropic" ? "default" : "outline"}
              onClick={() => setSelectedProvider("anthropic")}
            >
              Anthropic
            </Button>
            <Button
              variant={selectedProvider === "gemini" ? "default" : "outline"}
              onClick={() => setSelectedProvider("gemini")}
            >
              Gemini
            </Button>
          </div>
          {selectedProvider && (
            <Field label="API Key">
              <input
                type="password"
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder={`${selectedProvider} API Key`}
              />
            </Field>
          )}
          <p className="muted" style={{ marginTop: 10 }}>
            {t(
              "※API Keyは暗号化して保存されます。スキップして後から設定することも可能です。",
            )}
          </p>
        </div>
      ),
    },
    {
      title: t("ブラウザ機能の準備"),
      desc: t("AIがウェブを検索したり、ページを読んだりできるようにしますか？"),
      content: (
        <section className="card setup-browser-card" style={{ marginTop: 20 }}>
          <div className="setup-browser-icon">
            <Globe2 size={24} />
          </div>
          <div>
            <h2>Browser Tool</h2>
            <p>
              {t(
                "Playwright Chromiumを専用コンテナへ導入します。セットアップ完了後にバックグラウンドで始まるため、待たずに使い始められます。",
              )}
            </p>
            <label
              className="setup-browser-choice"
              aria-label={t("Browserを導入する")}
            >
              <input
                type="checkbox"
                checked={installBrowser}
                onChange={(event) => setInstallBrowser(event.target.checked)}
              />
              <span>
                <strong>{t("Browserを導入する")}</strong>
                <small>{t("Webページの閲覧と操作に使用します")}</small>
              </span>
            </label>
            <p className="muted">
              {t(
                "数百MBのダウンロードが必要です。後から設定画面で導入・再試行できます。",
              )}
            </p>
          </div>
        </section>
      ),
    },
    {
      title: t("準備完了"),
      desc: t("すべての設定が終わりました！"),
      content: (
        <div style={{ textAlign: "center", padding: "40px 0" }}>
          <Sparkles
            size={64}
            style={{ color: "var(--accent)", marginBottom: 20 }}
          />
          <h2>{t("さあ、始めましょう！")}</h2>
          <p style={{ color: "var(--muted)", marginTop: 10 }}>
            {t(
              "追加のモデル設定やMCPツールの追加は、左下の「設定」からいつでも行えます。",
            )}
          </p>
        </div>
      ),
    },
  ];

  return (
    <section className="setup-page">
      <div className="setup-shell">
        <header
          className="setup-header"
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "flex-start",
            marginBottom: 30,
          }}
        >
          <div>
            <p className="eyebrow">
              STEP {step + 1} OF {steps.length}
            </p>
            <h1 style={{ fontSize: 24, marginTop: 8 }}>
              {t(steps[step].title)}
            </h1>
            <p style={{ color: "var(--muted)", marginTop: 8 }}>
              {t(steps[step].desc)}
            </p>
          </div>
          <select
            aria-label={t("表示言語")}
            value={getLanguage()}
            onChange={(e) =>
              changeLanguage(e.target.value as "ja" | "en").catch(setError)
            }
            style={{
              padding: "4px 8px",
              borderRadius: 6,
              border: "1px solid var(--line)",
            }}
          >
            <option value="ja">{t("日本語")}</option>
            <option value="en">English</option>
          </select>
        </header>

        <div className="setup-content" style={{ flex: 1 }}>
          {steps[step].content}
        </div>

        <ErrorBox error={error} />

        {step === 0 && (
          <p className="setup-defer-note">
            {t(
              "「あとで設定する」を選ぶとセットアップは未完了のままです。ホームに案内が表示され、いつでもここへ戻れます。",
            )}
          </p>
        )}

        <div className="setup-footer">
          <Button
            variant="ghost"
            onClick={() => (step === 0 ? navigate("/") : setStep(step - 1))}
          >
            {step === 0 ? (
              t("あとで設定する")
            ) : (
              <>
                <ChevronLeft size={15} />
                {t("戻る")}
              </>
            )}
          </Button>

          {step < steps.length - 1 ? (
            <Button disabled={busy} onClick={handleNext}>
              {t(
                selectedProvider && !apiKey && step === 1 ? "スキップ" : "次へ",
              )}
              <ChevronRight size={15} />
            </Button>
          ) : (
            <Button disabled={busy} onClick={complete}>
              {t(busy ? "処理中…" : "完了して始める")}
              <Check size={15} />
            </Button>
          )}
        </div>
      </div>
    </section>
  );
}
