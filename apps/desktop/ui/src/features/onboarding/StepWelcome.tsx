import {
  ArrowRight,
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
export function StepWelcome({
  onDone,
  onDefer,
}: {
  onDone: () => void;
  onDefer: () => void;
}) {
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
    <div className="flex flex-col gap-8">
      <div className="flex flex-col gap-3">
        <span className="inline-flex w-fit items-center gap-1.5 rounded-full border border-line bg-surface px-2.5 py-1 text-2xs font-medium text-muted shadow-xs">
          <Sparkles size={11} className="text-brand" aria-hidden="true" />
          {t("初回セットアップ")}
        </span>
        <h2 className="text-3xl font-semibold tracking-tight text-fg">
          {t("ようこそ")}
        </h2>
        <p className="max-w-lg text-sm leading-6 text-muted">
          {t("ローカルで動く AI ワークスペースです。")}{" "}
          {t("3 つのステップで、すぐに使い始められます。")}
        </p>
      </div>

      <ul className="grid gap-3 sm:grid-cols-2">
        {CAPABILITIES.map(({ Icon, title, body }) => (
          <li
            key={title}
            className="flex items-start gap-3 rounded-xl border border-line bg-surface p-3.5 shadow-xs transition-[border-color,box-shadow] duration-150 hover:border-line-strong hover:shadow-sm"
          >
            <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-brand-soft text-brand">
              <Icon size={15} aria-hidden="true" />
            </span>
            <span className="flex min-w-0 flex-col gap-0.5">
              <span className="text-xs font-semibold text-fg">{t(title)}</span>
              <span className="text-xs leading-5 text-muted">{t(body)}</span>
            </span>
          </li>
        ))}
      </ul>

      <div className="flex items-start gap-3 rounded-xl border border-line bg-surface/60 p-4">
        <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-success-soft text-success">
          <ShieldCheck size={15} aria-hidden="true" />
        </span>
        <div className="flex min-w-0 flex-col gap-1">
          <span className="text-xs font-semibold text-fg">
            {t("ローカル実行と安全設計")}
          </span>
          <span className="text-xs leading-5 text-muted">
            {t(
              "会話履歴や設定はあなたの端末ローカルに安全に保存されます。ファイルの編集やシェル実行などの操作は、必ず事前に承認を求める安全設計になっています。",
            )}
          </span>
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <span className="text-2xs font-semibold uppercase tracking-[0.12em] text-faint">
          {t("表示言語")}
        </span>
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

      <div className="flex flex-wrap items-center gap-3 border-t border-line pt-6">
        <Button variant="primary" size="lg" onClick={onDone} className="group">
          {t("始める")}
          <ArrowRight
            size={14}
            className="transition-transform duration-150 group-hover:translate-x-0.5"
            aria-hidden="true"
          />
        </Button>
        <Button variant="ghost" size="lg" onClick={onDefer}>
          {t("あとで設定する")}
        </Button>
        <span className="text-xs text-faint">
          {t("設定はあとからいつでも変更できます。")}
        </span>
      </div>
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
