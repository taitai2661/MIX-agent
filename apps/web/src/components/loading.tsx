import { Loader2 } from "lucide-react";

export function Spinner({
  size = 16,
  className = "",
}: {
  size?: number;
  className?: string;
}) {
  return <Loader2 size={size} className={`spin ${className}`} />;
}

export function Skeleton({ className = "" }: { className?: string }) {
  return <div className={`skeleton ${className}`} />;
}

export function PageLoading({ message = "読み込み中…" }: { message?: string }) {
  return (
    <div className="page-loading">
      <Spinner size={24} />
      <p>{message}</p>
    </div>
  );
}

export function CardSkeleton() {
  return (
    <div className="card skeleton-card">
      <Skeleton className="skeleton-title" />
      <Skeleton className="skeleton-text" />
      <Skeleton className="skeleton-text short" />
    </div>
  );
}
