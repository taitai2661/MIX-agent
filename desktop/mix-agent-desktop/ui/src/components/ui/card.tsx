import { forwardRef } from "react";
import { cn } from "@/lib/cn";

export const Card = forwardRef<HTMLDivElement, React.ComponentProps<"div">>(
  function Card({ className, ...props }, ref) {
    return (
      <div
        ref={ref}
        className={cn(
          "rounded-lg border border-line bg-surface",
          "shadow-xs transition-colors duration-100 ease-out",
          className,
        )}
        {...props}
      />
    );
  },
);

export const CardTitle = forwardRef<HTMLDivElement, React.ComponentProps<"div">>(
  function CardTitle({ className, ...props }, ref) {
    return (
      <div
        ref={ref}
        className={cn(
          "flex items-center gap-1.5 px-3 py-2 text-sm font-semibold",
          "border-b border-line text-fg",
          className,
        )}
        {...props}
      />
    );
  },
);

export const CardBody = forwardRef<HTMLDivElement, React.ComponentProps<"div">>(
  function CardBody({ className, ...props }, ref) {
    return <div ref={ref} className={cn("p-3 text-sm", className)} {...props} />;
  },
);
