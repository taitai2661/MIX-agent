import { Keyboard } from "lucide-react";
import { t } from "@/lib/i18n";
import { modKey, shiftKey } from "@/lib/platform";
import { useUi } from "@/lib/ui";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { Kbd, SectionLabel } from "@/components/layout";

interface Shortcut {
  /** Key caps, rendered as separate chips in order. */
  keys: string[];
  /** Translation key (Japanese literal) for the action. */
  label: string;
}

interface ShortcutGroup {
  title: string;
  items: Shortcut[];
}

/**
 * The reference sheet for the app's keyboard shortcuts.
 *
 * These are display-only copies of the bindings in `useAppShortcuts` — the
 * handler needs live store setters and the sheet does not, and threading one
 * source through both bought less than it cost. The two are kept adjacent and
 * short enough to compare by eye; anything added to the handler belongs here.
 */
const GROUPS: ShortcutGroup[] = [
  {
    title: "移動",
    items: [
      { keys: ["Mod", "K"], label: "コマンドパレット" },
      { keys: ["Mod", "N"], label: "新規スレッド" },
      { keys: ["Mod", "B"], label: "サイドバー切り替え" },
      { keys: ["Mod", ","], label: "設定を開く" },
      { keys: ["Mod", "Shift", "E"], label: "ファイルツリー" },
      { keys: ["Mod", "Shift", "C"], label: "チャットに切り替え" },
      { keys: ["Mod", "Shift", "D"], label: "エージェントに切り替え" },
      { keys: ["?"], label: "この一覧を表示" },
    ],
  },
  {
    title: "入力",
    items: [
      // Replaced at render time with the user's send-key preference; keeping a
      // static "Enter 送信" here contradicted Settings when Modifier mode was on.
      { keys: ["Enter"], label: "送信" },
      { keys: ["Shift", "Enter"], label: "改行" },
      { keys: ["Esc"], label: "ダイアログを閉じる" },
    ],
  },
  {
    title: "表示",
    items: [
      { keys: ["Mod", "+"], label: "拡大" },
      { keys: ["Mod", "-"], label: "縮小" },
      { keys: ["Mod", "0"], label: "拡大率をリセット" },
    ],
  },
];

/** `?` (or the palette entry) opens this cheat sheet. */
export function ShortcutHelp() {
  const open = useUi((s) => s.shortcutsOpen);
  const setOpen = useUi((s) => s.setShortcutsOpen);
  const sendShortcut = useUi((s) => s.sendShortcut);
  const groups = GROUPS.map((group) => {
    if (group.title !== "入力") return group;
    return {
      ...group,
      items:
        sendShortcut === "modifier"
          ? [
              { keys: ["Mod", "Enter"], label: "送信" },
              { keys: ["Enter"], label: "改行" },
              { keys: ["Esc"], label: "ダイアログを閉じる" },
            ]
          : group.items,
    };
  });

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="max-w-lg">
        <DialogTitle className="flex items-center gap-2 pe-6">
          <Keyboard size={16} className="text-faint" aria-hidden="true" />
          {t("キーボードショートカット")}
        </DialogTitle>
        <div className="mt-5 flex flex-col gap-5">
          {groups.map((group) => (
            <section key={group.title} className="flex flex-col gap-2">
              <SectionLabel>{t(group.title)}</SectionLabel>
              <ul className="flex flex-col gap-1.5">
                {group.items.map((item) => (
                  <li
                    key={`${item.label}-${item.keys.join("-")}`}
                    className="flex items-center gap-3"
                  >
                    <span className="min-w-0 flex-1 text-sm text-muted">
                      {t(item.label)}
                    </span>
                    <span className="flex shrink-0 items-center gap-1">
                      {item.keys.map((key) => {
                        const cap =
                          key === "Mod"
                            ? modKey()
                            : key === "Shift"
                              ? shiftKey()
                              : key;
                        return <Kbd key={key}>{cap}</Kbd>;
                      })}
                    </span>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}
