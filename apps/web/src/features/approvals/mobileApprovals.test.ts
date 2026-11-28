import { afterEach, describe, expect, it, vi } from "vitest";

import {
  createServeClient,
  type MobilePendingApproval,
} from "./mobileApprovals";

const TOKEN = "tok+/=SECRET";
const client = () => createServeClient("https://serve.example", TOKEN);

function approval(id: string): MobilePendingApproval {
  return {
    approval_id: id,
    run_id: "r1",
    tool_call_id: "tc-1",
    tool_name: "terminal",
    arguments: {},
    created_at: "2026-01-01T00:00:00Z",
    expires_at: "2026-01-01T00:01:00Z",
  };
}

function sseResponse(...blocks: string[]): Response {
  const encoder = new TextEncoder();
  return new Response(
    new ReadableStream({
      start(controller) {
        for (const block of blocks) controller.enqueue(encoder.encode(block));
        controller.close();
      },
    }),
    { status: 200, headers: { "content-type": "text/event-stream" } },
  );
}

afterEach(() => vi.unstubAllGlobals());

describe("mobile serve client", () => {
  it("opens the SSE stream with an Authorization header, never a token query param", async () => {
    const fetchMock = vi.fn(async (_url: string, _init?: RequestInit) =>
      sseResponse(
        'event: replay\ndata: {"run_id":"r1","sequence":1,"kind":"x","data":{}}\n\n',
        'event: activity\ndata: {"run_id":"r1","sequence":2,"kind":"y","data":{}}\n\n',
        "event: done\ndata: {}\n\n",
      ),
    );
    vi.stubGlobal("fetch", fetchMock);
    const events: string[] = [];
    const onDone = vi.fn();
    const close = client().openRunEvents("r1", {
      onEvent: (event) => events.push(event.kind),
      onDone,
    });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalled());
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("https://serve.example/api/v1/runs/r1/events");
    expect(url).not.toContain("token=");
    expect(url).not.toContain(TOKEN);
    expect(init.headers).toEqual({ authorization: `Bearer ${TOKEN}` });
    expect(events).toEqual(["x", "y"]);
    close();
  });

  it("aborts the stream when the caller closes it", async () => {
    const fetchMock = vi.fn(
      (_url: string, _init?: RequestInit) =>
        new Promise<Response>(() => undefined),
    );
    vi.stubGlobal("fetch", fetchMock);
    const close = client().openRunEvents("r1", {});
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalled());
    const signal = (fetchMock.mock.calls[0] as [string, RequestInit])[1]
      .signal as AbortSignal;
    expect(signal.aborted).toBe(false);
    close();
    expect(signal.aborted).toBe(true);
  });

  it("scrubs the bearer token from server error messages", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              error: `rejected ${TOKEN} and ${encodeURIComponent(TOKEN)}`,
            }),
            { status: 403 },
          ),
      ),
    );
    await expect(client().fetchPendingApprovals()).rejects.toThrow(
      /rejected \[redacted\] and \[redacted\]/,
    );
    try {
      await client().fetchPendingApprovals();
    } catch (cause) {
      expect((cause as Error).message).not.toContain(TOKEN);
    }
  });

  it("only lets the newest poll update the inbox and aborts on stop", async () => {
    type Pending = {
      signal: AbortSignal | null | undefined;
      resolve: (response: Response) => void;
    };
    const pending: Pending[] = [];
    const fetchMock = vi.fn((_url: string, init?: RequestInit) => {
      let resolve!: (response: Response) => void;
      const response = new Promise<Response>((r) => (resolve = r));
      pending.push({ signal: init?.signal, resolve });
      return response;
    });
    vi.stubGlobal("fetch", fetchMock);

    const onApprovals = vi.fn();
    const poll = client().pollPending(onApprovals, vi.fn(), 60_000);
    expect(pending).toHaveLength(1);
    const first = pending[0];

    // An explicit SSE-triggered refresh supersedes the in-flight poll.
    poll.refreshNow();
    expect(first.signal?.aborted).toBe(true);
    expect(pending).toHaveLength(2);
    const second = pending[1];

    const respond = (approvals: MobilePendingApproval[]) =>
      new Response(JSON.stringify({ approvals }), { status: 200 });
    second.resolve(respond([approval("new")]));
    await vi.waitFor(() => expect(onApprovals).toHaveBeenCalledTimes(1));
    expect(onApprovals).toHaveBeenCalledWith([approval("new")]);

    // The aborted first response arrives late; it must not roll back the inbox.
    first.resolve(respond([approval("stale")]));
    await Promise.resolve();
    expect(onApprovals).toHaveBeenCalledTimes(1);

    // Starting another refresh then stopping aborts it and keeps the timer quiet.
    poll.refreshNow();
    expect(pending).toHaveLength(3);
    poll.stop();
    expect(pending[2].signal?.aborted).toBe(true);
    pending[2].resolve(respond([approval("after-stop")]));
    await Promise.resolve();
    expect(onApprovals).toHaveBeenCalledTimes(1);
  });

  it("posts approval decisions with the serve bridge's snake_case keys", async () => {
    const fetchMock = vi.fn(
      async (_url: string, _init?: RequestInit) =>
        new Response(JSON.stringify({ ok: true }), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await client().decideApproval("ap-1", {
      grant: true,
      remember: false,
      rememberSession: true,
      acknowledgeRisk: true,
    });

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("https://serve.example/api/v1/approvals/ap-1/decision");
    expect(init.method).toBe("POST");
    expect(JSON.parse(String(init.body))).toEqual({
      grant: true,
      remember: false,
      remember_session: true,
      acknowledge_risk: true,
    });
    expect(String(init.body)).not.toContain("rememberSession");
    expect(String(init.body)).not.toContain("acknowledgeRisk");
  });

  it("omits optional decision flags instead of sending undefined", async () => {
    const fetchMock = vi.fn(
      async (_url: string, _init?: RequestInit) =>
        new Response(JSON.stringify({ ok: true }), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await client().decideApproval("ap-2", { grant: false });

    const init = fetchMock.mock.calls[0][1] as RequestInit;
    expect(JSON.parse(String(init.body))).toEqual({ grant: false });
  });
});
