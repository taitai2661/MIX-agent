import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/cn";

const badgeVariants = cva(
  [
    "inline-flex items-center gap-1 rounded-md border font-medium",
    "whitespace-nowrap select-none tabular-nums",
  ],
  {
    variants: {
      tone: {
        // `border-line` (8% ink) all but disappears on `bg-surface` in light
        // mode: the chip and the card behind it are the same near-white. The
        // stronger hairline is what keeps a neutral chip reading as a shape
        // instead of a smudge of text. Tones that carry a soft fill already
        // have their own edge and keep the normal one.
        neutral: "border-line-strong bg-inset text-muted",
        brand: "border-transparent bg-brand-soft text-brand",
        success: "border-transparent bg-success-soft text-success",
        warning: "border-transparent bg-warning-soft text-warning",
        danger: "border-transparent bg-danger-soft text-danger",
        info: "border-transparent bg-info-soft text-info",
      },
      size: {
        sm: "h-4 px-1 text-2xs",
        md: "h-5 px-1.5 text-xs",
      },
    },
    defaultVariants: { tone: "neutral", size: "md" },
  },
);

export interface BadgeProps
  extends React.ComponentProps<"span">,
    VariantProps<typeof badgeVariants> {
  asChild?: boolean;
}

export function Badge({
  className,
  tone,
  size,
  asChild = false,
  ...props
}: BadgeProps) {
  const Comp = asChild ? Slot : "span";
  return (
    <Comp className={cn(badgeVariants({ tone, size }), className)} {...props} />
  );
}

export { badgeVariants };
