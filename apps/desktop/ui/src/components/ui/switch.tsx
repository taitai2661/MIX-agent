import * as SwitchPrimitive from "@radix-ui/react-switch";
import { forwardRef } from "react";
import { cn } from "@/lib/cn";

export const Switch = forwardRef<
  React.ComponentRef<typeof SwitchPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof SwitchPrimitive.Root>
>(function Switch({ className, ...props }, ref) {
  return (
    <SwitchPrimitive.Root
      ref={ref}
      className={cn(
        "peer inline-flex h-5 w-9 shrink-0 cursor-pointer items-center",
        "rounded-full border border-line-strong/50 bg-faint/40 p-0.5",
        "transition-colors duration-150 ease-out",
        "disabled:cursor-not-allowed disabled:opacity-45",
        "data-[state=checked]:border-transparent data-[state=checked]:bg-brand",
        "data-[state=unchecked]:hover:bg-faint/55",
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        className={cn(
          // `on-solid`, not a literal `white`: in dark mode the checked track is
          // a light tint, so the thumb flips to the dark token with it.
          "pointer-events-none block size-4 rounded-full bg-on-solid",
          "shadow-xs ring-1 ring-fg/10",
          "transition-transform duration-150 ease-out",
          "data-[state=checked]:translate-x-4",
        )}
      />
    </SwitchPrimitive.Root>
  );
});
