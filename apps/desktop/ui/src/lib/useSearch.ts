import { useCallback, useRef, useState } from "react";

/**
 * Shared async-search state for the Memory / Knowledge panes.
 *
 * Both panes hand-rolled the same quartet (`query` / `hits` / `searchError` /
 * `searching`) plus a `search()` that swallowed rejections into an empty list.
 * One hook owns the lifecycle so a failed search always surfaces its error
 * instead of rendering as "no hits".
 */
export function useSearch<T>(
  fn: (query: string, limit: number) => Promise<T[]>,
  limit = 20,
) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<T[] | null>(null);
  const [searchError, setSearchError] = useState<unknown>(null);
  const [searching, setSearching] = useState(false);
  const seq = useRef(0);
  // Mirrors `searching` for the submit guard: `run` must keep a stable
  // identity for the Enter-key handler, so `searching` stays out of its deps
  // and the ref is the only thing that sees mid-flight submits.
  const searchingRef = useRef(false);

  const run = useCallback(
    async (override?: string) => {
      const q = (override ?? query).trim();
      if (!q || searchingRef.current) return;
      if (override !== undefined) setQuery(override);
      const id = ++seq.current;
      setSearchError(null);
      searchingRef.current = true;
      setSearching(true);
      try {
        const rows = await fn(q, limit);
        // Rapid re-submits resolve out of order; only the latest wins.
        if (seq.current === id) setHits(rows);
      } catch (error) {
        if (seq.current === id) {
          setHits(null);
          setSearchError(error);
        }
      } finally {
        if (seq.current === id) {
          searchingRef.current = false;
          setSearching(false);
        }
      }
      // `searching` is read via `searchingRef`, not as a dependency: including
      // it would recreate `run` mid-flight and break the Enter-key handler's
      // identity.
    },
    [fn, limit, query],
  );

  const reset = useCallback(() => {
    seq.current += 1;
    searchingRef.current = false;
    setHits(null);
    setSearchError(null);
    setSearching(false);
  }, []);

  return { query, setQuery, hits, searchError, searching, run, reset };
}
