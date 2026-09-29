import { useRef, useState, type PointerEvent, type KeyboardEvent } from "react";
import { cn } from "@/lib/cn";

interface ResizableHandleProps {
  value: number;
  min: number;
  max: number;
  /** Positive when dragging right grows the pane; negative for right-side panes. */
  direction?: 1 | -1;
  label: string;
  onChange: (value: number) => void;
  onCommit: (value: number) => void;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function ResizableHandle({
  value,
  min,
  max,
  direction = 1,
  label,
  onChange,
  onCommit,
}: ResizableHandleProps) {
  const [dragging, setDragging] = useState(false);
  const drag = useRef<{
    x: number;
    width: number;
    currentWidth: number;
    pointerId: number;
  } | null>(null);

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = {
      x: event.clientX,
      width: value,
      currentWidth: value,
      pointerId: event.pointerId,
    };
    setDragging(true);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const start = drag.current;
    if (!start || start.pointerId !== event.pointerId) return;
    const next = clamp(start.width + (event.clientX - start.x) * direction, min, max);
    start.currentWidth = next;
    onChange(next);
  };

  const finishDrag = (event: PointerEvent<HTMLDivElement>) => {
    const start = drag.current;
    if (!start || start.pointerId !== event.pointerId) return;
    drag.current = null;
    setDragging(false);
    onCommit(start.currentWidth);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const step = event.shiftKey ? 32 : 12;
    const signed = event.key === "ArrowRight" ? step : -step;
    const next = clamp(value + signed * direction, min, max);
    onChange(next);
    onCommit(next);
  };

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={Math.round(value)}
      tabIndex={0}
      data-dragging={dragging}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={finishDrag}
      onPointerCancel={finishDrag}
      onKeyDown={onKeyDown}
      className={cn(
        "group relative z-10 w-1.5 shrink-0 cursor-col-resize touch-none outline-none",
        "after:absolute after:inset-y-0 after:left-1/2 after:w-px after:-translate-x-1/2",
        "after:bg-transparent after:transition-colors hover:after:bg-brand/50",
        "focus-visible:after:bg-brand data-[dragging=true]:after:bg-brand",
      )}
    />
  );
}
