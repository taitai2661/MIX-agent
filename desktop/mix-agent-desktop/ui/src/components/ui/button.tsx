import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { forwardRef } from "react";
import { cn } from "@/lib/cn";

const buttonVariants = cva(
  [
    "inline-flex items-center justify-center gap-1.5 whitespace-nowrap",
    "rounded-md font-medium select-none",
    "transition-[background-color,border-color,color,box-shadow,transform] duration-100 ease-out",
    // The focus ring itself lives in tokens.css as a global `:focus-visible`
    // rule, so it is deliberately *not* restated here. That rule applies to
    // every natively focusable element, which is why the hand-copied
    // `focus-visible:outline-2 …` strings that used to sit on a dozen call
    // sites could be deleted outright.
    "disabled:pointer-events-none disabled:opacity-45",
    // The one definition of "this option is picked". Six call sites used to
    // open-code this exact string on a `<Button>`, and another three spelled the
    // same state `bg-active` or with no border at all.
    "aria-pressed:bg-brand-soft aria-pressed:border-brand aria-pressed:text-brand",
  ],
  {
    variants: {
      variant: {
        default: [
          "bg-surface text-fg border border-line",
          "hover:bg-inset hover:border-line-strong active:scale-[0.98]",
        ],
        primary:
          "bg-brand text-on-solid hover:bg-brand-hover active:scale-[0.98]",
        // `on-solid`, not a literal `white`: in dark mode every solid fill in
        // the palette is a light tint, so text on top of one has to be dark.
        danger: "bg-danger text-on-solid hover:opacity-90 active:scale-[0.98]",
        // The destructive *quiet* action. Nine call sites were open-coding
        // `variant="ghost" className="text-danger hover:text-danger"`; as a
        // variant the danger tint is no longer optional, and it is a token
        // rather than a translucent overlay.
        "ghost-danger": "text-danger hover:bg-danger-soft hover:text-danger",
        ghost: "text-muted hover:bg-hover hover:text-fg",
        link: "text-brand underline-offset-4 hover:underline",
      },
      size: {
        sm: "h-8 px-3 text-xs",
        md: "h-control px-3.5 text-sm",
        lg: "h-10 px-5 text-sm",
        icon: "size-control shrink-0 p-0",
        "icon-sm": "size-6 shrink-0 p-0",
      },
    },
    defaultVariants: { variant: "default", size: "md" },
  },
);

export interface ButtonProps
  extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "color">,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
  /**
   * Marks a toggle button. Sets `aria-pressed` and the shared selected styling,
   * so callers no longer hand-pick a className for the state.
   */
  selected?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  function Button(
    { className, variant, size, asChild = false, type, selected, ...props },
    ref,
  ) {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        ref={ref}
        type={asChild ? undefined : (type ?? "button")}
        aria-pressed={selected}
        className={cn(buttonVariants({ variant, size }), className)}
        {...props}
      />
    );
  },
);

export { buttonVariants };
