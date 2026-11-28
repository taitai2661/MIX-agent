/// Minimal ANSI SGR handling shared by terminal surfaces.
///
/// The app does not ship an xterm canvas: tool results and the user terminal
/// render text with spans, so only SGR (colour / emphasis) is honoured. Every
/// other control sequence — cursor movement, erase, alternate screen, bracketed
/// paste, OSC titles — is stripped. The previous whitelist of CSI final bytes
/// missed `ESC[?25l`, `ESC[?1049h` and OSC, so those leaked into the transcript
/// as `[?25l` text.
///
/// Keeping one parser here means the agent's `terminal` results and the user's
/// "ターミナル" tab colour output the same way instead of one of them silently
/// dropping the escape codes.

export interface AnsiSpan {
  text: string;
  className: string;
}

/** Every escape sequence except SGR, which is parsed separately. */
const NON_SGR_ESCAPES =
  // eslint-disable-next-line no-control-regex
  /\x1b\](?:[^\x07\x1b]*(?:\x07|\x1b\\)|[^\x07\x1b]*$)|(?!\x1b\[[0-9;]*m)\x1b\[[0-?]*[ -/]*[@-~]|\x1b\[[0-?]*[ -/]*$|\x1b(?!\[|\])[\x20-\x7e]/g;
/** SGR only (`ESC[...m`); everything else has already been removed. */
// eslint-disable-next-line no-control-regex
const SGR = /\x1b\[([0-9;]*)m/g;

/** Remove every escape sequence, leaving plain text. */
export function stripAnsi(raw: string): string {
  return raw.replace(NON_SGR_ESCAPES, "").replace(SGR, "");
}

/**
 * Normalise terminal output into the text a span-based view can show.
 *
 * - Non-SGR control sequences are removed (cursor, erase, alternate screen,
 *   bracketed paste, OSC titles).
 * - A bare carriage return rewrites the current line, so only the text after
 *   the last `\r` survives. Turning each `\r` into `\n` (the old behaviour)
 *   turned a single progress bar into one line per update.
 */
export function normalizeTerminalText(raw: string): string {
  return raw
    .replace(NON_SGR_ESCAPES, "")
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((line) => {
      const lastReturn = line.lastIndexOf("\r");
      return lastReturn === -1 ? line : line.slice(lastReturn + 1);
    })
    .join("\n");
}

/** Codes 30–37 / 90–97 (foreground) mapped to the app's semantic tokens. */
const FOREGROUND: Record<number, string> = {
  30: "text-faint",
  90: "text-faint",
  31: "text-danger",
  91: "text-danger",
  32: "text-success",
  92: "text-success",
  33: "text-warning",
  93: "text-warning",
  34: "text-brand",
  94: "text-brand",
  35: "text-ansi-magenta",
  95: "text-ansi-magenta",
  36: "text-ansi-cyan",
  96: "text-ansi-cyan",
  37: "text-fg",
  97: "text-fg",
};

/** Background colours (40–47 / 100–107); only the semantic soft tints exist. */
const BACKGROUND: Record<number, string> = {
  41: "bg-danger-soft",
  101: "bg-danger-soft",
  42: "bg-success-soft",
  102: "bg-success-soft",
  43: "bg-warning-soft",
  103: "bg-warning-soft",
  44: "bg-brand-soft",
  104: "bg-brand-soft",
  46: "bg-info-soft",
  106: "bg-info-soft",
};

interface SgrState {
  foreground: string | null;
  background: string | null;
  bold: boolean;
  dim: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
}

function resetState(state: SgrState): void {
  state.foreground = null;
  state.background = null;
  state.bold = false;
  state.dim = false;
  state.italic = false;
  state.underline = false;
  state.strike = false;
}

/**
 * Apply one SGR parameter list. Extended colour forms (`38;5;n` and
 * `38;2;r;g;b`) are consumed so their numbers are not mistaken for separate
 * attributes; the app has no fixed 256-colour palette, so those stay uncoloured
 * rather than picking a wrong semantic token.
 */
function applySgr(params: string, state: SgrState): void {
  const codes = (params || "0").split(";").map(Number);
  for (let index = 0; index < codes.length; index += 1) {
    const code = codes[index];
    if (code === 38 || code === 48) {
      const mode = codes[index + 1];
      if (mode === 5) index += 2;
      else if (mode === 2) index += 4;
      continue;
    }
    if (!Number.isFinite(code)) continue;
    if (code === 0) resetState(state);
    else if (code === 1) state.bold = true;
    else if (code === 2) state.dim = true;
    else if (code === 3) state.italic = true;
    else if (code === 4) state.underline = true;
    else if (code === 9) state.strike = true;
    else if (code === 22) state.bold = state.dim = false;
    else if (code === 23) state.italic = false;
    else if (code === 24) state.underline = false;
    else if (code === 29) state.strike = false;
    else if (code === 39) state.foreground = null;
    else if (code === 49) state.background = null;
    else if (FOREGROUND[code]) state.foreground = FOREGROUND[code];
    else if (BACKGROUND[code]) state.background = BACKGROUND[code];
  }
}

function classNameFor(state: SgrState): string {
  return [
    state.foreground,
    state.background,
    state.bold ? "font-bold" : "",
    state.dim ? "opacity-70" : "",
    state.italic ? "italic" : "",
    state.underline ? "underline" : "",
    state.strike ? "line-through" : "",
  ]
    .filter(Boolean)
    .join(" ");
}

/**
 * Parse text containing SGR codes into styled spans. Every non-SGR escape is
 * removed first, so the returned text is safe to render directly.
 */
export function parseAnsi(raw: string): AnsiSpan[] {
  const cleaned = normalizeTerminalText(raw);
  const spans: AnsiSpan[] = [];
  const state: SgrState = {
    foreground: null,
    background: null,
    bold: false,
    dim: false,
    italic: false,
    underline: false,
    strike: false,
  };

  const push = (text: string) => {
    if (text) spans.push({ text, className: classNameFor(state) });
  };

  let last = 0;
  SGR.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = SGR.exec(cleaned)) !== null) {
    push(cleaned.slice(last, match.index));
    applySgr(match[1] ?? "", state);
    last = match.index + match[0].length;
  }
  push(cleaned.slice(last));

  return spans;
}
