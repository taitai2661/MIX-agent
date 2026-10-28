export interface FrameRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface FrameSize {
  width: number;
  height: number;
}

/**
 * Map a click on the rendered `<img>` back to CSS pixels in the page.
 *
 * The image is displayed with `object-contain`, so the element box can be
 * wider/taller than the drawn picture (letterboxing). Measuring the element
 * box alone — the old implementation — drifts by the letterbox and clicks the
 * wrong element as soon as the pane's aspect ratio differs from the viewport.
 * The drawn rect is derived from the intrinsic size and the box, and clicks
 * outside it are ignored.
 */
export function framePoint(
  clientX: number,
  clientY: number,
  rect: FrameRect,
  natural: FrameSize,
  page: FrameSize,
): { x: number; y: number } | null {
  if (
    rect.width <= 0 ||
    rect.height <= 0 ||
    natural.width <= 0 ||
    natural.height <= 0 ||
    page.width <= 0 ||
    page.height <= 0
  ) {
    return null;
  }
  const scale = Math.min(
    rect.width / natural.width,
    rect.height / natural.height,
  );
  const drawnWidth = natural.width * scale;
  const drawnHeight = natural.height * scale;
  if (drawnWidth <= 0 || drawnHeight <= 0) return null;

  const offsetX = rect.left + (rect.width - drawnWidth) / 2;
  const offsetY = rect.top + (rect.height - drawnHeight) / 2;
  const ratioX = (clientX - offsetX) / drawnWidth;
  const ratioY = (clientY - offsetY) / drawnHeight;
  if (ratioX < 0 || ratioX > 1 || ratioY < 0 || ratioY > 1) return null;

  return {
    x: Math.min(page.width - 1, Math.max(0, Math.round(ratioX * page.width))),
    y: Math.min(page.height - 1, Math.max(0, Math.round(ratioY * page.height))),
  };
}
