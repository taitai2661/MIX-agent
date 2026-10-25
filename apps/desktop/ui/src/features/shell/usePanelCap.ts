import { useEffect, useState } from "react";

/** Width the centre column keeps even when every side panel is at its floor. */
const CENTER_MIN = 300;
/** Smallest share a panel may be squeezed to before the others stop giving. */
const PANEL_MIN = 160;

/**
 * How wide the side panels are *allowed* to be, given the row they share with
 * the centre column.
 *
 * The sidebar, the file tree and the right panel are all `shrink-0`, and each
 * one remembers its own width in localStorage. Nothing re-negotiated them, so
 * opening all three in a small window (880px is the minimum: 264 + 240 + 380)
 * pushed their total past the window edge — the centre column collapsed to
 * zero and every toolbar inside it spilled out of the window.
 *
 * Each pane marks itself with `data-side-panel` and applies the returned number
 * as `max-width`, so the remembered width survives and comes back as soon as
 * there is room again. `null` means they already fit and nothing needs capping.
 *
 * `row` is the shell row the panels live in, tracked as state through a
 * callback ref: the shell renders a spinner until the onboarding gate resolves,
 * so the element only appears on a later render and a plain `useRef` would be
 * read while it is still null.
 */
export function usePanelCap(row: HTMLElement | null): number | null {
  const [cap, setCap] = useState<number | null>(null);

  useEffect(() => {
    if (!row) return;
    // Coalesce measurement into one animation frame.
    //
    // A mutation observer on the subtree fires for every node a streaming run
    // appends (a tool row, a token of markdown), and `measureCap` reads layout
    // (`getBoundingClientRect`, `querySelectorAll`) — so measuring per mutation
    // forced a reflow storm exactly while the transcript was busiest. The cap
    // only depends on panel visibility and the row's width, which a single
    // frame is more than enough to absorb.
    let frame = 0;
    const measure = () => {
      if (frame) return;
      frame = window.requestAnimationFrame(() => {
        frame = 0;
        setCap(measureCap(row));
      });
    };
    measure();
    // The window. Both of these matter: `resize` is what a native window
    // actually delivers, and a ResizeObserver on the row is what catches the
    // row being resized by something other than the window.
    window.addEventListener("resize", measure);
    const sizes = new ResizeObserver(measure);
    sizes.observe(row);
    // Which panels exist: opening or closing one does not change the row's own
    // size, so neither of the above hears about it and the cap would be left at
    // the value computed for the previous set.
    const contents = new MutationObserver(measure);
    contents.observe(row, { childList: true, subtree: true });
    return () => {
      if (frame) window.cancelAnimationFrame(frame);
      window.removeEventListener("resize", measure);
      sizes.disconnect();
      contents.disconnect();
    };
  }, [row]);

  return cap;
}

/** Inline style for a panel: its remembered width, capped at `cap`. */
export function panelStyle(
  width: number,
  cap: number | null,
): React.CSSProperties {
  return { width, maxWidth: cap ?? undefined };
}

/** What a panel actually renders at, which is what a drag handle reports. */
export function panelWidth(width: number, cap: number | null): number {
  return cap == null ? width : Math.min(width, cap);
}

function measureCap(row: HTMLElement): number | null {
  const budget = row.clientWidth - CENTER_MIN;
  const rowLeft = row.getBoundingClientRect().left;
  const widths: number[] = [];

  for (const panel of row.querySelectorAll<HTMLElement>("[data-side-panel]")) {
    const rect = panel.getBoundingClientRect();
    // A closed sidebar is pushed out with a negative margin rather than
    // unmounted, so it still measures — but it reserves no width.
    if (rect.width === 0 || rect.right <= rowLeft + 1) continue;
    // The width the panel asked for, not the one it rendered: the cap is
    // applied as `max-width`, so measuring the rendered width would shrink
    // the budget on every pass and never settle.
    const requested = Number.parseFloat(panel.style.width);
    widths.push(Number.isFinite(requested) ? requested : rect.width);
  }

  if (widths.length === 0) return null;
  const wanted = widths.reduce((sum, width) => sum + width, 0);
  if (wanted <= budget) return null;
  // The panels give up the shortfall in equal shares, so no single one of them
  // has to absorb the whole squeeze.
  return Math.max(PANEL_MIN, Math.floor(budget / widths.length));
}
