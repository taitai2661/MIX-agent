import * as PopoverPrimitive from "@radix-ui/react-popover";
import { forwardRef } from "react";
import { cn } from "@/lib/cn";

/**
 * Anchored, non-modal surface.
 *
 * Used by the model picker, which needs a searchable list *and* a text input —
 * a `DropdownMenu` moves focus with the arrow keys and owns the keyboard, so a
 * search field inside one fights the menu's own navigation. `Popover` leaves
 * the contents to the caller and only owns positioning, outside-click and
 * Escape.
 */
export const Popover = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;
export const PopoverAnchor = PopoverPrimitive.Anchor;

export const PopoverContent = forwardRef<
  React.ComponentRef<typeof PopoverPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof PopoverPrimitive.Content>
>(function PopoverContent(
  { className, align = "start", sideOffset = 6, ...props },
  ref,
) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        ref={ref}
        align={align}
        sideOffset={sideOffset}
        collisionPadding={8}
        className={cn(
          "z-50 overflow-hidden rounded-xl border border-line bg-elevated shadow-md",
          "animate-fade outline-none",
          className,
        )}
        {...props}
      />
    </PopoverPrimitive.Portal>
  );
});
