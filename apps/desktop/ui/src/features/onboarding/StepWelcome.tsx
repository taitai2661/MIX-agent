import {
  FolderTree,
  Globe,
  MessageSquare,
  ShieldCheck,
  Sparkles,
} from "lucide-react";
import { t } from "@/lib/i18n";
import { useApplySettings } from "@/lib/settings";
import { useUi } from "@/lib/ui";
import type { Language } from "@/lib/i18n";
import { Row } from "@/components/layout";
import { Button } from "@/components/ui/button";

/** Japanese is the canonical key — `t()` renders the translation when the UI
 * language is English. Held as literals and translated at render, not at module
 * scope: a module-level `t()` captures the language that happened to be active
 * when the bundle was first evaluated and never updates. */
const CAPABILITIES = [
  { Icon: MessageSquare, title: "チャット", body: "質問、対談、アイデア出し" },
  { Icon: Sparkles, title: "思考モード", body: "長い推論の過程をテーブルで" },
  {
    Icon: FolderTree,
    title: "エージェント",
    body: "フォルダの中のファイルを読み書き",
  },
  { Icon: Globe, title: "ブラウザ操作", body: "実際にブラウザを操作して検証" },
];

/**
 * Step 0: what this is, and a language choice.
 *
 * The language toggle is here rather than buried in Settings because a first-run
 * wizard is the one moment the user is certain about wanting to read the app in
 * their language — and the rest of the wizard would otherwise be in whatever
 * `navigator.languages` guessed.
 */
export function StepWelcome({ onDone }: { onDone: () => void }) {
  const locale = useUi((s) => s.locale);
  const setLocale = useUi((s) => s.setLocale);
  const applySettings = useApplySettings();

  const chooseLanguage = (language: Language) => {
    setLocale(language);
    // Both sides, like the sidebar's language control: `setLocale` only writes
    // localStorage, and `App` re-syncs from the backend at boot — a store-only
    // change would be reverted on the next launch.
    void applySettings({ language });
  };

  return (
    <div className="flex flex-col gap-5">
      <div className="grid size-12 place-items-center rounded-2xl border border-line bg-gradient-to-b from-surface to-inset text-brand">
        <Sparkles size={22} aria-hidden="true" />
      </div>
      <div className="flex flex-col gap-1.5">
        <h2 className="text-xl font-semibold tracking-tight text-fg">
          {t("ようこそ")}
        </h2>
        <p className="text-sm text-muted">
          {t("ローカルで動く AI ワークスペースです。")}
        </p>
      </div>

      <ul className="grid grid-cols-2 gap-3">
        {CAPABILITIES.map(({ Icon, title, body }) => (
          <li
            key={title}
            className="flex items-start gap-2.5 rounded-xl border border-line bg-surface px-3 py-3 transition-colors hover:border-line-strong"
          >
            <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-brand-soft text-brand">
              <Icon size={14} aria-hidden="true" />
            </span>
            <span className="flex flex-col gap-0.5">
              <span className="text-xs font-medium text-fg">{t(title)}</span>
              <span className="text-xs text-faint">{t(body)}</span>
            </span>
          </li>
        ))}
      </ul>

      <div className="flex items-start gap-2.5 rounded-xl border border-line bg-surface/60 px-3.5 py-3">
        <ShieldCheck
          size={16}
          className="mt-0.5 shrink-0 text-brand"
          aria-hidden="true"
        />
        <div className="flex flex-col gap-0.5 text-xs">
          <span className="font-medium text-fg">
            {t("ローカル実行と安全設計")}
          </span>
          <span className="text-muted leading-relaxed">
            {t(
              "会話履歴や設定はあなたの端末ローカルに安全に保存されます。ファイルの編集やシェル実行などの操作は、必ず事前に承認を求める安全設計になっています。",
            )}
          </span>
        </div>
      </div>

      <div className="flex flex-col gap-1.5">
        <span className="text-xs text-faint">{t("表示言語")}</span>
        <Row>
          <LanguageChoice
            language="ja"
            active={locale === "ja"}
            onChoose={chooseLanguage}
          />
          <LanguageChoice
            language="en"
            active={locale === "en"}
            onChoose={chooseLanguage}
          />
        </Row>
      </div>

      <Row>
        <Button variant="primary" size="lg" onClick={onDone}>
          {t("始める")}
        </Button>
        <span className="text-xs text-faint">
          {t("設定はあとからいつでも変更できます。")}
        </span>
      </Row>
    </div>
  );
}

/** One half of the language choice.
 *
 *  A pressed button, not a segmented control: picking a language is a
 *  persistent setting rather than one of several views of the same data, and
 *  the two options are labelled in their own language rather than the UI's. */
function LanguageChoice({
  language,
  active,
  onChoose,
}: {
  language: Language;
  active: boolean;
  onChoose: (language: Language) => void;
}) {
  return (
    <Button size="sm" selected={active} onClick={() => onChoose(language)}>
      {language === "ja" ? "日本語" : "English"}
    </Button>
  );
}
