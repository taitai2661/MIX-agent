import { useEffect, useState } from "react";

/** Width the centre column keeps even when every side panel is at its floor. */
const CENTER_MIN = 300;
/** Smallest share a panel may be squeezed to before the others stop giving. */
const PANEL_MIN = 160;
/** Hard floor for the right (browser) panel when the window is very narrow. */
const RIGHT_PANEL_MIN = 320;

/** What the shell row can give each side panel. */
export interface PanelBounds {
  /** Equal-share cap for the left panes (sidebar / file tree). */
  cap: number | null;
  /** Maximum the right pane may use while keeping the centre column usable. */
  rightMax: number | null;
  /** Current row width in CSS pixels (0 until the first measurement). */
  rowWidth: number;
}

const EMPTY_BOUNDS: PanelBounds = { cap: null, rightMax: null, rowWidth: 0 };

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
 * Left panes keep the equal-share `cap`. The right pane gets its own
 * `rightMax`: it is the pane the user expands to read a page, so it may take
 * every pixel the centre does not need instead of being limited to an equal
 * share. That is what makes the browser split feel free (Codex-style) rather
 * than stepping through 380 / 560 presets.
 *
 * `row` is the shell row the panels live in, tracked as state through a
 * callback ref: the shell renders a spinner until the onboarding gate resolves,
 * so the element only appears on a later render and a plain `useRef` would be
 * read while it is still null.
 */
export function usePanelCap(row: HTMLElement | null): PanelBounds {
  const [bounds, setBounds] = useState<PanelBounds>(EMPTY_BOUNDS);

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
        setBounds(measureCap(row));
      });
    };
    // Which panels exist and how wide they are. `ResizeObserver` on the row
    // alone misses a pane being dragged narrower or wider, because the row
    // itself keeps its size. Re-observe on child changes so opening (and
    // unmounting) a pane does not leave a stale observer behind.
    const panes = new ResizeObserver(measure);
    const observePanes = () => {
      panes.disconnect();
      for (const pane of row.querySelectorAll<HTMLElement>(
        "[data-side-panel]",
      )) {
        panes.observe(pane);
      }
    };
    observePanes();
    measure();
    // The window. Both of these matter: `resize` is what a native window
    // actually delivers, and a ResizeObserver on the row is what catches the
    // row being resized by something other than the window.
    window.addEventListener("resize", measure);
    const sizes = new ResizeObserver(measure);
    sizes.observe(row);
    // Opening or closing a panel does not change the row's own size, so
    // neither of the above hears about it and the cap would be left at the
    // value computed for the previous set.
    const contents = new MutationObserver(() => {
      observePanes();
      measure();
    });
    contents.observe(row, { childList: true, subtree: true });
    return () => {
      if (frame) window.cancelAnimationFrame(frame);
      window.removeEventListener("resize", measure);
      sizes.disconnect();
      panes.disconnect();
      contents.disconnect();
    };
  }, [row]);

  return bounds;
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

function measureCap(row: HTMLElement): PanelBounds {
  const rowWidth = row.clientWidth;
  const budget = rowWidth - CENTER_MIN;
  const rowLeft = row.getBoundingClientRect().left;
  const widths: number[] = [];
  let otherWidths = 0;
  let otherCount = 0;

  for (const panel of row.querySelectorAll<HTMLElement>("[data-side-panel]")) {
    // The right panel can be an absolute overlay (the default) instead of a
    // docked sibling. It still carries `data-side-panel`, but out-of-flow
    // panes reserve no inline width and must not shrink the sidebar/file tree
    // when they open.
    if (getComputedStyle(panel).position === "absolute") continue;
    const rect = panel.getBoundingClientRect();
    // A closed sidebar is pushed out with a negative margin rather than
    // unmounted, so it still measures — but it reserves no width.
    if (rect.width === 0 || rect.right <= rowLeft + 1) continue;
    // The width the panel asked for, not the one it rendered: the cap is
    // applied as `max-width`, so measuring the rendered width would shrink
    // the budget on every pass and never settle.
    const requested = Number.parseFloat(panel.style.width);
    const wanted = Number.isFinite(requested) ? requested : rect.width;
    if (panel.dataset.sidePanel === "right") {
      // The right pane is handled by `rightMax` below.
    } else {
      otherWidths += wanted;
      otherCount += 1;
    }
    widths.push(wanted);
  }

  // The right pane may use everything except the centre's floor and the other
  // panes' requested widths. This is what lets a 50% split work even when the
  // sidebar is open; the equal-share cap would have stopped at 44%.
  const rightMax =
    rowWidth > 0
      ? Math.max(
          RIGHT_PANEL_MIN,
          Math.floor(rowWidth - CENTER_MIN - otherWidths),
        )
      : null;

  if (widths.length === 0) return { cap: null, rightMax, rowWidth };

  // Left panes still give up an equal shortfall when they alone overflow.
  const leftCap =
    otherWidths > budget
      ? Math.max(PANEL_MIN, Math.floor(budget / Math.max(1, otherCount)))
      : null;
  return { cap: leftCap, rightMax, rowWidth };
}
