import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearNotificationTarget,
  consumeNotificationTarget,
  notifyWithTarget,
} from "./notifications";

const isPermissionGranted = vi.fn(async () => true);
const sendNotification = vi.fn();

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: () => isPermissionGranted(),
  sendNotification: (options: unknown) => sendNotification(options),
}));

beforeEach(() => {
  clearNotificationTarget();
  isPermissionGranted.mockResolvedValue(true);
  sendNotification.mockClear();
});

describe("notification targets", () => {
  it("records a destination that the next focus can consume once", async () => {
    const ok = await notifyWithTarget({
      title: "MIX-agent",
      body: "done",
      target: { kind: "thread", conversationId: "c1" },
    });
    expect(ok).toBe(true);
    expect(sendNotification).toHaveBeenCalled();
    expect(consumeNotificationTarget()).toEqual({
      kind: "thread",
      conversationId: "c1",
    });
    expect(consumeNotificationTarget()).toBeNull();
  });

  it("expires a destination after two minutes", async () => {
    await notifyWithTarget({
      title: "MIX-agent",
      body: "approval",
      target: { kind: "approval", conversationId: "c2" },
    });
    expect(consumeNotificationTarget(Date.now() + 3 * 60 * 1000)).toBeNull();
  });

  it("does not record anything when permission is denied", async () => {
    isPermissionGranted.mockResolvedValueOnce(false);
    const ok = await notifyWithTarget({
      title: "MIX-agent",
      body: "done",
      target: { kind: "thread", conversationId: "c3" },
    });
    expect(ok).toBe(false);
    expect(sendNotification).not.toHaveBeenCalled();
    expect(consumeNotificationTarget()).toBeNull();
  });
});
