import { useEffect, useRef, useState } from "react";

/** Follow `value`, but publish it to the caller at most once per `ms`.
 *
 * Token streams arrive every ~60 ms. Rendering a Markdown document re-parses
 * the whole accumulated text each time, so a long answer got progressively
 * more expensive to draw. Throttling the *render* (while the store still
 * applies every event) keeps the transcript responsive without losing text:
 * the trailing timeout always publishes the latest value.
 */
export function useThrottledValue<T>(value: T, ms: number): T {
  const [published, setPublished] = useState(value);
  const latest = useRef(value);
  latest.current = value;
  const lastPublished = useRef(0);
  const timer = useRef<number | null>(null);

  useEffect(() => {
    // A reset (a new Run in the same thread) must be visible immediately;
    // throttling it would briefly show the previous answer's tail.
    if (ms <= 0 || (typeof value === "string" && value.length === 0)) {
      lastPublished.current = Date.now();
      setPublished(value);
      return;
    }
    const now = Date.now();
    const elapsed = now - lastPublished.current;
    if (elapsed >= ms) {
      lastPublished.current = now;
      setPublished(value);
      return;
    }
    if (timer.current !== null) return;
    timer.current = window.setTimeout(
      () => {
        timer.current = null;
        lastPublished.current = Date.now();
        setPublished(latest.current);
      },
      // At least one frame, so a burst of updates cannot schedule a
      // zero-delay timeout that runs before the browser paints.
      Math.max(ms - elapsed, 16),
    );
  }, [value, ms]);

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );

  return published;
}
