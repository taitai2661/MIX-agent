import { Slot } from "@radix-ui/react-slot";
import { forwardRef } from "react";
import { ChevronDown } from "lucide-react";
import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import { cn } from "@/lib/cn";

export const DropdownMenu = DropdownMenuPrimitive.Root;
export const DropdownMenuTrigger = DropdownMenuPrimitive.Trigger;

export const DropdownMenuContent = forwardRef<
  React.ComponentRef<typeof DropdownMenuPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Content>
>(function DropdownMenuContent({ className, sideOffset = 4, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.Content
        ref={ref}
        sideOffset={sideOffset}
        collisionPadding={8}
        className={cn(
          "z-50 min-w-44 overflow-hidden rounded-xl border border-line bg-elevated p-1 shadow-md",
          "animate-fade",
          className,
        )}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  );
});

const itemClass = [
  "relative flex cursor-pointer select-none items-center gap-2",
  "rounded-md px-2 py-1.5 text-sm text-fg outline-none",
  "transition-colors duration-75",
  "data-[highlighted]:bg-active",
  "data-[disabled]:pointer-events-none data-[disabled]:opacity-45",
].join(" ");

export const DropdownMenuItem = forwardRef<
  React.ComponentRef<typeof DropdownMenuPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Item> & {
    /** Two-line item: a label and a dimmed description beneath it. Used by the
     *  mode, model, surface and project pickers, which all had to hand-write the
     *  stacking with `flex-col items-start gap-0 py-1.5`. */
    description?: React.ReactNode;
  }
>(function DropdownMenuItem(
  { className, description, children, ...props },
  ref,
) {
  if (description == null) {
    return (
      <DropdownMenuPrimitive.Item
        ref={ref}
        className={cn(itemClass, className)}
        {...props}
      >
        {children}
      </DropdownMenuPrimitive.Item>
    );
  }
  return (
    <DropdownMenuPrimitive.Item
      ref={ref}
      className={cn(itemClass, "flex-col items-start gap-0 py-1.5", className)}
      {...props}
    >
      <span className="flex w-full items-center gap-2">{children}</span>
      <span className="ps-0 text-xs text-faint">{description}</span>
    </DropdownMenuPrimitive.Item>
  );
});

export const DropdownMenuLabel = forwardRef<
  React.ComponentRef<typeof DropdownMenuPrimitive.Label>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Label>
>(function DropdownMenuLabel({ className, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.Label
      ref={ref}
      className={cn("px-2 py-1.5 text-2xs font-medium text-faint", className)}
      {...props}
    />
  );
});

export const DropdownMenuSeparator = forwardRef<
  React.ComponentRef<typeof DropdownMenuPrimitive.Separator>,
  React.ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Separator>
>(function DropdownMenuSeparator({ className, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.Separator
      ref={ref}
      className={cn("-mx-1 my-1 h-px bg-line", className)}
      {...props}
    />
  );
});

/**
 * Trigger button for a dropdown.
 *
 * The composer's mode, model and project pickers plus the surface switcher all
 * retyped the same control — an `h-8` bordered pill with a label, an icon slot
 * and a chevron — as a literal utility string, four times in one file.
 */
export const DropdownMenuTriggerButton = forwardRef<
  HTMLButtonElement,
  React.ComponentPropsWithoutRef<"button"> & { asChild?: boolean }
>(function DropdownMenuTriggerButton(
  { className, children, asChild, ...props },
  ref,
) {
  const Comp = asChild ? Slot : "button";
  return (
    <Comp
      ref={ref}
      type="button"
      className={cn(
        "inline-flex h-control max-w-56 min-w-0 items-center gap-1.5 rounded-md border border-line",
        "bg-surface px-2.5 text-xs font-medium text-fg",
        "transition-colors duration-100 hover:bg-hover",
        "data-[state=open]:border-line-strong",
        className,
      )}
      {...props}
    >
      {children}
      <ChevronDown
        size={11}
        className="shrink-0 text-muted"
        aria-hidden="true"
      />
    </Comp>
  );
});
