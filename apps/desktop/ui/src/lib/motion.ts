/// Whether JS-driven motion (smooth scroll, animated jumps) should be skipped.
///
/// CSS animations are handled by the `data-reduce-motion` rule in tokens.css.
/// `scrollTo({ behavior: "smooth" })` is not: it needs a runtime check, so it
/// consults both the user's in-app override and the OS preference.

import { useUi } from "./ui";

export function prefersReducedMotion(): boolean {
  if (useUi.getState().reduceMotion) return true;
  return (
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true
  );
}
