import { useMemo } from "react";
import { cn } from "@/lib/cn";
import { parseAnsi } from "@/lib/ansi";

/**
 * Render terminal output with SGR colours as spans.
 *
 * Use this wherever PTY output is shown as text (the user terminal and the
 * agent's `terminal` tool result). Non-SGR control sequences are stripped so a
 * progress bar does not leave escape garbage in the transcript.
 */
export function AnsiText({
  text,
  className,
  empty,
}: {
  text: string;
  className?: string;
  /** Shown when `text` is blank; omit to render nothing. */
  empty?: React.ReactNode;
}) {
  // `parseAnsi` normalises control sequences itself; doing it twice was wasted
  // work on every streamed terminal chunk.
  const spans = useMemo(() => parseAnsi(text), [text]);
  if (spans.length === 0) return <>{empty ?? null}</>;
  return (
    <span className={className}>
      {spans.map((span, index) => (
        <span key={index} className={span.className || undefined}>
          {span.text}
        </span>
      ))}
    </span>
  );
}

/** Convenience for `pre` output blocks that keep whitespace. */
export function AnsiPre({
  text,
  className,
}: {
  text: string;
  className?: string;
}) {
  return (
    <pre className={cn("whitespace-pre-wrap break-words", className)}>
      <AnsiText text={text} />
    </pre>
  );
}
