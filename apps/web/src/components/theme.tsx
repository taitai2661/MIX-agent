import { Laptop, Moon, Sun } from "lucide-react";
import { useEffect, useState } from "react";

export type Theme = "light" | "dark" | "system";
const key = "mix-agent-theme";

function initialTheme(): Theme {
  const stored = localStorage.getItem(key);
  return stored === "light" || stored === "dark" || stored === "system"
    ? stored
    : "system";
}

export function ThemeController() {
  const [theme, setTheme] = useState<Theme>(initialTheme);
  useEffect(() => {
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      document.documentElement.dataset.theme =
        theme === "system" ? (query.matches ? "dark" : "light") : theme;
    };
    apply();
    query.addEventListener("change", apply);
    localStorage.setItem(key, theme);
    window.dispatchEvent(new CustomEvent("mix-agent-theme", { detail: theme }));
    return () => query.removeEventListener("change", apply);
  }, [theme]);
  useEffect(() => {
    const sync = (event: Event) => setTheme((event as CustomEvent<Theme>).detail);
    window.addEventListener("mix-agent-theme", sync);
    return () => window.removeEventListener("mix-agent-theme", sync);
  }, []);
  return <ThemeToggle theme={theme} onChange={setTheme} />;
}

function ThemeToggle({ theme, onChange }: { theme: Theme; onChange: (theme: Theme) => void }) {
  const options: [Theme, typeof Sun, string][] = [
    ["light", Sun, "ライト"],
    ["dark", Moon, "ダーク"],
    ["system", Laptop, "システム"],
  ];
  return (
    <div className="theme-toggle" aria-label="テーマ">
      {options.map(([value, Icon, label]) => (
        <button
          aria-label={label}
          aria-pressed={theme === value}
          className={theme === value ? "selected" : ""}
          key={value}
          onClick={() => onChange(value)}
          type="button"
        >
          <Icon size={15} />
        </button>
      ))}
    </div>
  );
}
