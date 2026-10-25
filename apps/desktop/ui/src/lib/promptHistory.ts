import { getJSON, setJSON } from "./storage";

const PROMPT_HISTORY_KEY = "mix:prompt-history";
const MAX_PROMPT_HISTORY = 50;

function isStringArray(val: unknown): val is string[] {
  return Array.isArray(val) && val.every((item) => typeof item === "string");
}

/** Load prompt history (most recent first). */
export function loadPromptHistory(store?: Storage | null): string[] {
  return getJSON<string[]>(PROMPT_HISTORY_KEY, [], isStringArray, store);
}

/** Push a prompt to history (most recent first, deduplicated with immediate previous). */
export function pushPromptHistory(
  prompt: string,
  store?: Storage | null,
): string[] {
  const trimmed = prompt.trim();
  if (!trimmed) return loadPromptHistory(store);

  const prev = loadPromptHistory(store);
  // Deduplicate if identical to the most recent one
  const filtered = prev.filter((item, idx) =>
    idx === 0 ? item !== trimmed : true,
  );
  const next = [trimmed, ...filtered.filter((item) => item !== trimmed)].slice(
    0,
    MAX_PROMPT_HISTORY,
  );
  setJSON(PROMPT_HISTORY_KEY, next, store);
  return next;
}
