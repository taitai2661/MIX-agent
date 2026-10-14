import { forwardRef, type ReactNode } from "react";
import { X } from "lucide-react";
import { cn } from "@/lib/cn";
import { Button } from "./button";

const control = [
  "w-full min-w-0 rounded-md border border-line bg-surface px-2.5 text-sm text-fg",
  "placeholder:text-faint",
  "transition-colors duration-100 ease-out",
  "hover:border-line-strong",
  // The border carries focus, not a ring: a `focus:` rule (rather than
  // `focus-visible:`) is deliberate here because clicking into a field has to
  // say "you are in here" too. The token is the app's single focus colour.
  "focus:border-brand focus:ring-2 focus:ring-brand/15 focus:outline-none",
  "disabled:opacity-45 disabled:cursor-not-allowed",
].join(" ");

export const Input = forwardRef<HTMLInputElement, React.ComponentProps<"input">>(
  function Input({ className, ...props }, ref) {
    return <input ref={ref} className={cn(control, "h-control", className)} {...props} />;
  },
);

export const Textarea = forwardRef<HTMLTextAreaElement, React.ComponentProps<"textarea">>(
  function Textarea({ className, ...props }, ref) {
    return (
      <textarea
        ref={ref}
        className={cn(control, "min-h-16 resize-y py-1.5 leading-5", className)}
        {...props}
      />
    );
  },
);

/**
 * Wraps a native `<select>`. Deliberately not a Radix listbox: the app has
 * ~20 of these inside a Tauri WebView, and native gives correct OS keyboard
 * handling and a real popup for free.
 */
/**
 * Search box: leading icon, borderless input, optional trailing clear.
 *
 * The sidebar's thread filter hand-typed the whole control — including a copy of
 * `control`'s border, radius, padding and hover rule — and then had to opt its
 * own `focus-within:border-brand` in because a borderless inner input cannot
 * show focus on its own. The wrapper owns the border, the input owns the text.
 */
export const SearchField = forwardRef<
  HTMLInputElement,
  Omit<React.ComponentProps<"input">, "className"> & {
  icon?: ReactNode;
  onClear?: () => void;
  /** Only needed alongside `onClear`; required so the clear button always has
   *  an accessible name, which is the whole reason it is not a bare glyph. */
  clearLabel?: string;
  }
>(function SearchField({ icon, onClear, clearLabel, ...props }, ref) {
  return (
    <div
      className={cn(
        "flex h-control items-center gap-2 rounded-md border border-line bg-surface px-2.5",
        "transition-colors duration-100 ease-out hover:border-line-strong",
        // The wrapper is what shows focus, since the input inside is borderless.
        "focus-within:border-brand focus-within:ring-2 focus-within:ring-brand/15",
      )}
    >
      {icon && <span className="shrink-0 text-faint">{icon}</span>}
      <input
        ref={ref}
        className="min-w-0 flex-1 bg-transparent text-sm text-fg outline-none placeholder:text-faint"
        {...props}
      />
      {onClear && (
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={onClear}
          title={clearLabel}
          aria-label={clearLabel}
        >
          <X size={11} />
        </Button>
      )}
    </div>
  );
});

/** Chevron drawn as a mask so its fill is `currentColor` and follows the theme
 *  (the previous data-URI hardcoded the light-mode `--color-faint` hex, which
 *  left a light-grey arrow on a dark field in dark mode). */
const chevron = {
  WebkitMaskImage:
    "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='m6 9 6 6 6-6'/%3E%3C/svg%3E\")",
  maskImage:
    "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='m6 9 6 6 6-6'/%3E%3C/svg%3E\")",
  WebkitMaskRepeat: "no-repeat",
  maskRepeat: "no-repeat",
  WebkitMaskPosition: "right 8px center",
  maskPosition: "right 8px center",
  WebkitMaskSize: "12px 12px",
  maskSize: "12px 12px",
} as const;

export const Select = forwardRef<HTMLSelectElement, React.ComponentProps<"select">>(
  function Select({ className, ...props }, ref) {
    return (
      <select
        ref={ref}
        className={cn(
          control,
          "h-control cursor-pointer appearance-none pr-7 text-fg",
          "has-[option:checked]:text-fg",
          className,
        )}
        style={chevron}
        {...props}
      />
    );
  },
);
