// @vitest-environment jsdom
/// Risk gating for the web inline approval card. Interactive DOM rendering is
/// needed here (renderToStaticMarkup cannot exercise checkbox/button state).
import { act, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { getLanguage, setLanguage, type Language } from "@/app/i18n";
import { RunActivity } from "./RunActivity";
import type { Approval, ToolCallHistory } from "./types";

Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });

let container: HTMLDivElement;
let root: Root;
let previousLanguage: Language;

beforeEach(() => {
  previousLanguage = getLanguage();
  setLanguage("ja");
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  setLanguage(previousLanguage);
});

function render(ui: ReactElement) {
  act(() => root.render(ui));
}

function call(approvalId = "ap-1"): ToolCallHistory {
  return {
    id: "call-1",
    run_id: "run-1",
    status: "waiting_approval",
    tool_name: "run_terminal",
    created_at: "2026-01-01T00:00:00Z",
    retry: { available: false, label: "" },
    approval: { id: approvalId, status: "pending" },
  };
}

function approval(id: string, risk?: string): Approval {
  return {
    id,
    status: "pending",
    created_at: "2026-01-01T00:00:00Z",
    data: {
      tool: "run_terminal",
      arguments: { command: "rm -rf build" },
      scope: {},
      ...(risk ? { risk } : {}),
    },
  };
}

function button(label: string): HTMLButtonElement {
  const found = Array.from(container.querySelectorAll("button")).find(
    (node) => node.textContent === label,
  );
  if (!found) throw new Error(`button not found: ${label}`);
  return found as HTMLButtonElement;
}

function card(): HTMLElement {
  const found = container.querySelector(".run-step-approval");
  if (!found) throw new Error("approval card not found");
  return found as HTMLElement;
}

describe("web approval risk gating", () => {
  it("hides always-allow for a write risk and gates once on acknowledgement", async () => {
    const onApproval = vi.fn(async () => undefined);
    render(
      <RunActivity
        calls={[call()]}
        approvals={[approval("ap-1", "write")]}
        onApproval={onApproval}
      />,
    );

    expect(card().textContent).not.toContain("この操作を常に許可");
    expect(card().textContent).toContain("変更を伴う操作");
    const once = button("今回のみ");
    expect(once.disabled).toBe(true);
    // Denying is always available: the user must be able to back out without
    // first acknowledging the risk.
    expect(button("拒否").disabled).toBe(false);

    const checkbox = card().querySelector<HTMLInputElement>(
      'input[type="checkbox"]',
    );
    expect(checkbox).not.toBeNull();
    expect(checkbox?.closest("label")?.textContent).toContain(
      "内容を理解しました",
    );

    act(() => {
      checkbox?.click();
    });
    expect(button("今回のみ").disabled).toBe(false);
    await act(async () => {
      button("今回のみ").click();
    });
    expect(onApproval).toHaveBeenCalledWith("ap-1", "once", true);
  });

  it("blocks a risky grant until the checkbox is ticked, but never blocks deny", async () => {
    const onApproval = vi.fn(async () => undefined);
    render(
      <RunActivity
        calls={[call()]}
        approvals={[approval("ap-1", "write")]}
        onApproval={onApproval}
      />,
    );
    // The disabled button cannot submit even when a click is forced.
    await act(async () => {
      button("今回のみ").click();
    });
    expect(onApproval).not.toHaveBeenCalled();

    // Deny needs no acknowledgement and carries no flag.
    await act(async () => {
      button("拒否").click();
    });
    expect(onApproval).toHaveBeenCalledWith("ap-1", "denied", undefined);
  });

  it.each(["external", "dangerous"])(
    "also gates %s approvals",
    async (risk) => {
      const onApproval = vi.fn(async () => undefined);
      render(
        <RunActivity
          calls={[call()]}
          approvals={[approval("ap-1", risk)]}
          onApproval={onApproval}
        />,
      );
      expect(card().textContent).not.toContain("この操作を常に許可");
      expect(button("今回のみ").disabled).toBe(true);
      act(() => {
        card()
          .querySelector<HTMLInputElement>('input[type="checkbox"]')!
          .click();
      });
      await act(async () => {
        button("今回のみ").click();
      });
      expect(onApproval).toHaveBeenCalledWith("ap-1", "once", true);
    },
  );

  it("keeps the legacy controls and plain payload for read-only approvals", async () => {
    const onApproval = vi.fn(async () => undefined);
    render(
      <RunActivity
        calls={[call()]}
        approvals={[approval("ap-1", "read")]}
        onApproval={onApproval}
      />,
    );
    expect(card().textContent).toContain("この操作を常に許可");
    expect(
      card().querySelector<HTMLInputElement>('input[type="checkbox"]'),
    ).toBeNull();
    expect(button("今回のみ").disabled).toBe(false);
    await act(async () => {
      button("この操作を常に許可").click();
    });
    expect(onApproval).toHaveBeenCalledWith("ap-1", "always", undefined);
  });

  it("resets acknowledgement when a new approval replaces the old one", () => {
    render(
      <RunActivity
        calls={[call("ap-1")]}
        approvals={[approval("ap-1", "write")]}
        onApproval={() => {}}
      />,
    );
    const firstCheckbox = card().querySelector<HTMLInputElement>(
      'input[type="checkbox"]',
    )!;
    act(() => firstCheckbox.click());
    expect(button("今回のみ").disabled).toBe(false);

    // runEvents replaces next.approval while the call (and its component) stays
    // mounted; the second command must start unacknowledged.
    render(
      <RunActivity
        calls={[call("ap-2")]}
        approvals={[approval("ap-2", "external")]}
        onApproval={() => {}}
      />,
    );
    expect(button("今回のみ").disabled).toBe(true);
  });

  it("drops acknowledgement when the same approval id changes its arguments", async () => {
    const base = approval("ap-1", "write");
    const changed: Approval = {
      ...base,
      data: { ...base.data, arguments: { command: "rm -rf /" } },
    };
    const onApproval = vi.fn(async () => undefined);
    render(
      <RunActivity
        calls={[call("ap-1")]}
        approvals={[base]}
        onApproval={onApproval}
      />,
    );
    const checkbox = card().querySelector<HTMLInputElement>(
      'input[type="checkbox"]',
    )!;
    act(() => checkbox.click());
    expect(button("今回のみ").disabled).toBe(false);

    render(
      <RunActivity
        calls={[call("ap-1")]}
        approvals={[changed]}
        onApproval={onApproval}
      />,
    );
    expect(
      card().querySelector<HTMLInputElement>('input[type="checkbox"]')?.checked,
    ).toBe(false);
    expect(button("今回のみ").disabled).toBe(true);
    await act(async () => button("今回のみ").click());
    expect(onApproval).not.toHaveBeenCalled();
  });

  it("is accessible: labelled checkbox, real buttons, unique ids, no dangling aria", () => {
    render(
      <RunActivity
        calls={[call()]}
        approvals={[approval("ap-1", "write")]}
        onApproval={() => {}}
      />,
    );
    const checkbox = card().querySelector<HTMLInputElement>(
      'input[type="checkbox"]',
    )!;
    expect(checkbox.closest("label")).not.toBeNull();
    expect(checkbox.getAttribute("aria-hidden")).toBeNull();
    for (const node of Array.from(
      card().querySelectorAll("[aria-describedby]"),
    )) {
      const id = node.getAttribute("aria-describedby")!;
      expect(document.getElementById(id)).not.toBeNull();
    }
    const ids = Array.from(card().querySelectorAll("[id]")).map((n) => n.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const b of Array.from(card().querySelectorAll("button"))) {
      expect(b.textContent?.trim().length).toBeGreaterThan(0);
    }
  });
});
