import { cva, type VariantProps } from "class-variance-authority";
import { AlertTriangle, CircleAlert, CircleCheck, Info } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/lib/cn";

const alertVariants = cva(
  "flex items-start gap-2 rounded-lg border border-transparent px-3 py-2 text-sm",
  {
    variants: {
      tone: {
        info: "bg-info-soft text-info",
        success: "bg-success-soft text-success",
        warning: "bg-warning-soft text-warning",
        danger: "bg-danger-soft text-danger",
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
  /** Set when `children` already contain a leading icon, so the alert does not
   *  render a second one. A verification-failure alert used to show both the
   *  tone's `CircleAlert` and its own `ShieldAlert`. */
  hideIcon?: boolean;
}

export function Alert({
  className,
  tone = "info",
  action,
  hideIcon = false,
  children,
  ...props
}: AlertProps) {
  const Icon = ICONS[tone ?? "info"];
  return (
    <div
      role={tone === "danger" ? "alert" : "status"}
      className={cn(alertVariants({ tone }), className)}
      {...props}
    >
      {!hideIcon && <Icon size={14} className="mt-0.5 shrink-0" aria-hidden="true" />}
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
