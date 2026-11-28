/**
 * Mobile approval-inbox client sketch for `mixd serve` (A7/P3-4).
 *
 * Talks to the serve HTTP bridge (`apps/desktop/app/src/http/mod.rs`), NOT to
 * the Docker `apps/web` backend in `src/app/api.ts`: auth here is a Bearer
 * token (`MIX_SERVE_TOKEN`), there are no cookies/CSRF, and the error envelope
 * is `{ error }`. Keep this module free of `@/` imports so the sketch can
 * move into the PWA inbox route without dragging the desktop-web store along.
 *
 * Intended phone flow:
 *  1. `fetchPendingApprovals()` on launch and every 5 s (fallback poll).
 *  2. `openRunEvents()` per visible run: `tool_approval_requested` refreshes
 *     the inbox immediately, `tool_approval_resolved` / terminal `status`
 *     dismisses the card.
 *  3. `decideApproval()` on tap. `dangerous` prompts need
 *     `acknowledgeRisk: true` (the server rejects otherwise) and must never
 *     send `remember: true`.
 */

export interface MobileRisk {
  level?: string;
  [key: string]: unknown;
}

export interface MobilePendingApproval {
  approval_id: string;
  run_id: string;
  tool_call_id: string;
  tool_name: string;
  arguments: unknown;
  created_at: string;
  expires_at: string;
  risk?: MobileRisk | null;
}

export interface MobileDecision {
  grant: boolean;
  remember?: boolean;
  rememberSession?: boolean;
  acknowledgeRisk?: boolean;
}

export type MobileRunEvent = {
  run_id: string;
  sequence: number;
  kind: string;
  data: Record<string, unknown>;
};

export interface MobileAnswer {
  runId: string;
  questionId?: string;
  selected?: string[];
  freeText?: string;
}

export interface MobileRunDetail {
  id: string;
  conversation_id?: string | null;
  status: string;
  mode?: unknown;
  modelId?: unknown;
  updatedAt?: unknown;
}

export function createServeClient(baseUrl: string, token: string) {
  /** Never let the bearer secret escape into a log, toast, or rejection text. */
  function redact(value: string): string {
    return value
      .split(token)
      .join("[redacted]")
      .split(encodeURIComponent(token))
      .join("[redacted]");
  }
  function safeError(cause: unknown): Error {
    const message = cause instanceof Error ? cause.message : String(cause);
    return new Error(redact(message));
  }

  async function request<T>(path: string, init?: RequestInit): Promise<T> {
    let response: Response;
    try {
      response = await fetch(baseUrl + path, {
        ...init,
        headers: {
          authorization: `Bearer ${token}`,
          ...(init?.method === "POST"
            ? { "content-type": "application/json" }
            : {}),
          ...init?.headers,
        },
      });
    } catch (cause) {
      throw safeError(cause);
    }
    if (!response.ok) {
      let message = `serve request failed (${response.status})`;
      try {
        const body = (await response.json()) as { error?: unknown };
        if (typeof body.error === "string" && body.error) message = body.error;
      } catch {
        /* keep the status-based message */
      }
      throw new Error(redact(message));
    }
    return (await response.json()) as T;
  }

  /** Full inbox snapshot. Call on launch, on reconnect, and on a 5 s poll. */
  function fetchPendingApprovals(
    signal?: AbortSignal,
  ): Promise<MobilePendingApproval[]> {
    return request<{ approvals: MobilePendingApproval[] }>(
      "/api/v1/approvals/pending",
      {
        signal,
      },
    ).then((body) => body.approvals ?? []);
  }

  /**
   * Answer one prompt. The server mirrors the desktop guards, so a client bug
   * cannot launder a dangerous command into "always allow": such requests are
   * rejected with 422 and surface here as thrown Errors.
   *
   * The serve HTTP bridge reads snake_case only (`http/mod.rs::decide`), so
   * translate the camelCase public shape here. Sending `acknowledgeRisk` /
   * `rememberSession` verbatim would silently drop both flags: a dangerous
   * grant would always 422 and a session grant would degrade to "once".
   */
  function decideApproval(
    approvalId: string,
    decision: MobileDecision,
  ): Promise<void> {
    const body: {
      grant: boolean;
      remember?: boolean;
      remember_session?: boolean;
      acknowledge_risk?: boolean;
    } = { grant: decision.grant };
    if (decision.remember !== undefined) body.remember = decision.remember;
    if (decision.rememberSession !== undefined)
      body.remember_session = decision.rememberSession;
    if (decision.acknowledgeRisk !== undefined)
      body.acknowledge_risk = decision.acknowledgeRisk;
    return request<{ ok: boolean }>(
      `/api/v1/approvals/${encodeURIComponent(approvalId)}/decision`,
      { method: "POST", body: JSON.stringify(body) },
    ).then(() => undefined);
  }

  /**
   * Single run header (no transcript; open `openRunEvents` for replay+live).
   * Unknown ids throw (404) so the inbox can drop stale cards.
   */
  function fetchRunDetail(
    runId: string,
    signal?: AbortSignal,
  ): Promise<MobileRunDetail> {
    return request<MobileRunDetail>(
      `/api/v1/runs/${encodeURIComponent(runId)}`,
      {
        signal,
      },
    );
  }

  /**
   * Answer one `ask_user` card. Mirrors desktop `answer_question`:
   * formatted via `format_question_answer`, delivered as steering.
   */
  function answerQuestion(answer: MobileAnswer): Promise<boolean> {
    return request<{ ok: boolean; accepted: boolean }>("/api/v1/answers", {
      method: "POST",
      body: JSON.stringify({
        run_id: answer.runId,
        question_id: answer.questionId,
        selected: answer.selected ?? [],
        free_text: answer.freeText,
      }),
    }).then((body) => body.accepted);
  }

  /** Signal cancellation for a live Run (idempotent). */
  function cancelRun(runId: string): Promise<void> {
    return request<{ ok: boolean }>(
      `/api/v1/runs/${encodeURIComponent(runId)}/cancel`,
      { method: "POST", body: JSON.stringify({}) },
    ).then(() => undefined);
  }

  /**
   * Live event stream for one run. The server replays up to 1000 recent
   * events as `event: replay` first, then streams live `activity`.
   * Callers must still `fetchPendingApprovals()` first for prompts.
   */
  function openRunEvents(
    runId: string,
    handlers: {
      onEvent?: (event: MobileRunEvent) => void;
      onDone?: () => void;
      onError?: (cause: unknown) => void;
    },
  ): () => void {
    // EventSource cannot set an Authorization header, and `?token=` leaks the
    // bearer secret into proxy/history logs. The serve SSE route also accepts
    // header auth (see `http/mod.rs`), so read the stream with fetch instead.
    const controller = new AbortController();
    let closed = false;
    const close = () => {
      if (closed) return;
      closed = true;
      controller.abort();
    };
    void (async () => {
      try {
        const response = await fetch(
          `${baseUrl}/api/v1/runs/${encodeURIComponent(runId)}/events`,
          {
            headers: { authorization: `Bearer ${token}` },
            signal: controller.signal,
          },
        );
        if (!response.ok || !response.body) {
          throw new Error(`approval stream failed (${response.status})`);
        }
        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let buffer = "";
        const dispatch = (block: string) => {
          let event = "message";
          const dataLines: string[] = [];
          for (const line of block.split("\n")) {
            if (line.startsWith("event:")) event = line.slice(6).trim();
            else if (line.startsWith("data:"))
              dataLines.push(line.slice(5).replace(/^ /, ""));
          }
          if (!dataLines.length) return;
          if (event === "done") {
            handlers.onDone?.();
            return;
          }
          if (event !== "replay" && event !== "activity") return;
          try {
            handlers.onEvent?.(
              JSON.parse(dataLines.join("\n")) as MobileRunEvent,
            );
          } catch (cause) {
            handlers.onError?.(cause);
          }
        };
        while (!closed) {
          const { value, done } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          let end: number;
          while ((end = buffer.indexOf("\n\n")) >= 0) {
            const block = buffer.slice(0, end);
            buffer = buffer.slice(end + 2);
            if (!closed) dispatch(block);
          }
        }
        if (!closed) handlers.onDone?.();
      } catch (cause) {
        if (!closed) handlers.onError?.(safeError(cause));
      }
    })();
    return close;
  }

  /**
   * Polling fallback with the same "refresh on activity" shape as the desktop
   * `useChatRunStream` invalidations: poll every `intervalMs`, and expose
   * `refreshNow()` for SSE-triggered updates.
   */
  function pollPending(
    onApprovals: (approvals: MobilePendingApproval[]) => void,
    onError: (cause: unknown) => void,
    intervalMs = 5_000,
  ): { refreshNow: () => void; stop: () => void } {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let inFlight: AbortController | null = null;
    // Monotonic generation: a late response from an aborted/superseded refresh
    // can never overwrite a newer inbox snapshot.
    let generation = 0;
    const refreshNow = () => {
      if (stopped) return;
      if (inFlight) {
        // An explicit refresh (SSE signal) supersedes the older poll rather
        // than being dropped by the in-flight guard.
        inFlight.abort();
        inFlight = null;
      }
      const controller = new AbortController();
      const requestId = ++generation;
      inFlight = controller;
      fetchPendingApprovals(controller.signal)
        .then((approvals) => {
          if (!stopped && requestId === generation) onApprovals(approvals);
        })
        .catch((cause: unknown) => {
          if (!stopped && requestId === generation) onError(safeError(cause));
        })
        .finally(() => {
          if (inFlight === controller) inFlight = null;
        });
    };
    const tick = () => {
      if (stopped) return;
      // A slow poll must not stack a second request on top of itself.
      if (!inFlight) refreshNow();
      timer = setTimeout(tick, intervalMs);
    };
    tick();
    return {
      refreshNow,
      stop: () => {
        stopped = true;
        generation += 1;
        inFlight?.abort();
        inFlight = null;
        if (timer !== undefined) clearTimeout(timer);
      },
    };
  }

  return {
    fetchPendingApprovals,
    fetchRunDetail,
    decideApproval,
    answerQuestion,
    cancelRun,
    openRunEvents,
    pollPending,
  };
}

export type ServeClient = ReturnType<typeof createServeClient>;
