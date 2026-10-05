import { Keyboard } from "lucide-react";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { Kbd } from "@/components/layout";

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
      { keys: ["⌘", "K"], label: "コマンドパレット" },
      { keys: ["⌘", "N"], label: "新規スレッド" },
      { keys: ["⌘", "⇧", "E"], label: "ファイルツリー" },
      { keys: ["⌘", "⇧", "C"], label: "チャットサーフェスへ" },
      { keys: ["⌘", "⇧", "D"], label: "エージェントサーフェスへ" },
      { keys: ["?"], label: "この一覧を表示" },
    ],
  },
  {
    title: "入力",
    items: [
      { keys: ["Enter"], label: "送信" },
      { keys: ["⇧", "Enter"], label: "改行" },
      { keys: ["Esc"], label: "ダイアログを閉じる" },
    ],
  },
];

/** `?` (or the palette entry) opens this cheat sheet. */
export function ShortcutHelp() {
  const open = useUi((s) => s.shortcutsOpen);
  const setOpen = useUi((s) => s.setShortcutsOpen);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="max-w-lg">
        <DialogTitle className="flex items-center gap-2 pe-6">
          <Keyboard size={16} className="text-faint" aria-hidden="true" />
          {t("キーボードショートカット")}
        </DialogTitle>
        <div className="mt-5 flex flex-col gap-5">
          {GROUPS.map((group) => (
            <section key={group.title} className="flex flex-col gap-2">
              <h3 className="text-2xs font-semibold tracking-[0.08em] text-faint uppercase">
                {t(group.title)}
              </h3>
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
                      {item.keys.map((key) => (
                        <Kbd key={key}>{key}</Kbd>
                      ))}
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
