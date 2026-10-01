import { t } from "@/app/i18n";
import { Check, ChevronDown } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";

export type SelectOption = {
  value: string;
  label: string;
  description?: string;
  icon?: ReactNode;
};

export function SelectMenu({
  value,
  onChange,
  options,
  placeholder,
  disabled,
  icon,
  ariaLabel,
  className,
}: {
  value: string;
  onChange: (value: string) => void;
  options: SelectOption[];
  placeholder?: string;
  disabled?: boolean;
  icon?: ReactNode;
  ariaLabel?: string;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const selected = options.find((option) => option.value === value);

  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const index = options.findIndex((option) => option.value === value);
    setActive(index < 0 ? 0 : index);
    // Only re-sync the highlighted option when the menu opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  function choose(option: SelectOption) {
    onChange(option.value);
    setOpen(false);
    trigger.current?.focus();
  }

  function keyboard(event: React.KeyboardEvent) {
    if (event.key === "Escape") {
      setOpen(false);
      trigger.current?.focus();
      return;
    }
    if (!open) {
      if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        setOpen(true);
      }
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActive((index) => Math.min(index + 1, options.length - 1));
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive((index) => Math.max(index - 1, 0));
    }
    if (event.key === "Enter" && options[active]) {
      event.preventDefault();
      choose(options[active]);
    }
  }

  return (
    <div className={"select-menu" + (className ? " " + className : "")} ref={root} onKeyDown={keyboard}>
      <button
        ref={trigger}
        type="button"
        className="select-trigger"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        disabled={disabled}
        onClick={() => setOpen((shown) => !shown)}
      >
        {icon}
        <span>{selected?.label || placeholder || t("選択")}</span>
        <ChevronDown size={15} />
      </button>
      {open && (
        <div className="select-menu-pop" role="listbox" aria-label={ariaLabel}>
          {options.map((option, index) => (
            <button
              key={option.value}
              type="button"
              role="option"
              aria-selected={option.value === value}
              className={index === active ? "active" : ""}
              onMouseEnter={() => setActive(index)}
              onClick={() => choose(option)}
            >
              {option.icon && <span className="select-option-icon">{option.icon}</span>}
              <span className="select-option-text">
                <b>{option.label}</b>
                {option.description && <small>{option.description}</small>}
              </span>
              {option.value === value && <Check size={15} />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
