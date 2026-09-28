import { forwardRef } from "react";
import { cn } from "@/lib/cn";

const control = [
  "w-full min-w-0 rounded-md border border-line bg-surface px-2.5 text-sm text-fg",
  "placeholder:text-faint",
  "transition-colors duration-100 ease-out",
  "hover:border-line-strong",
  "focus:border-brand focus:outline-none focus:ring-2 focus:ring-focus",
  "disabled:opacity-45 disabled:cursor-not-allowed",
].join(" ");

export const Input = forwardRef<HTMLInputElement, React.ComponentProps<"input">>(
  function Input({ className, ...props }, ref) {
    return <input ref={ref} className={cn(control, "h-8", className)} {...props} />;
  },
);

export const Textarea = forwardRef<HTMLTextAreaElement, React.ComponentProps<"textarea">>(
  function Textarea({ className, ...props }, ref) {
    return (
      <textarea
        ref={ref}
        className={cn(control, "resize-y py-1.5 leading-5", className)}
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
export const Select = forwardRef<HTMLSelectElement, React.ComponentProps<"select">>(
  function Select({ className, ...props }, ref) {
    return (
      <select
        ref={ref}
        className={cn(control, "h-8 cursor-pointer appearance-none pr-7", className)}
        style={{
          backgroundImage:
            "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%238f949c' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='m6 9 6 6 6-6'/%3E%3C/svg%3E\")",
          backgroundRepeat: "no-repeat",
          backgroundPosition: "right 8px center",
        }}
        {...props}
      />
    );
  },
);
