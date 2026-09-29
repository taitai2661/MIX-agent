import { cva, type VariantProps } from "class-variance-authority";
import { AlertTriangle, CircleAlert, CircleCheck, Info } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/lib/cn";

const alertVariants = cva(
  "flex items-start gap-2 rounded-lg border px-3 py-2 text-sm",
  {
    variants: {
      tone: {
        info: "border-transparent bg-info-soft text-info",
        success: "border-transparent bg-success-soft text-success",
        warning: "border-transparent bg-warning-soft text-warning",
        danger: "border-transparent bg-danger-soft text-danger",
      },
    },
    defaultVariants: { tone: "info" },
  },
);

const ICONS = {
  info: Info,
  success: CircleCheck,
  warning: AlertTriangle,
  danger: CircleAlert,
} as const;

export interface AlertProps
  extends React.ComponentProps<"div">,
    VariantProps<typeof alertVariants> {
  action?: ReactNode;
}

export function Alert({ className, tone = "info", action, children, ...props }: AlertProps) {
  const Icon = ICONS[tone ?? "info"];
  return (
    <div
      role={tone === "danger" ? "alert" : "status"}
      className={cn(alertVariants({ tone }), className)}
      {...props}
    >
      <Icon size={14} className="mt-0.5 shrink-0" />
      <div className="min-w-0 flex-1">{children}</div>
      {action}
    </div>
  );
}

export function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      aria-hidden
      className={cn("animate-pulse rounded-md bg-inset", className)}
      {...props}
    />
  );
}
