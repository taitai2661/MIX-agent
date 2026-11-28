import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { setLanguage } from "@/lib/i18n";
import { createTimeline } from "@/lib/runEvents";
import { useRunEvents } from "@/lib/runStore";
import { SteeringQueue } from "./SteeringQueue";

const listSteering = vi.fn(async () => [{ seq: 1, content: "修正して" }]);
const updateSteering = vi.fn(
  async (_runId: string, _seq: number, _content: string) => true,
);
const removeSteering = vi.fn(async (_runId: string, _seq: number) => true);

vi.mock("@/lib/api", () => ({
  api: {
    listSteering: () => listSteering(),
    updateSteering: (runId: string, seq: number, content: string) =>
      updateSteering(runId, seq, content),
    removeSteering: (runId: string, seq: number) => removeSteering(runId, seq),
  },
}));

beforeAll(() => setLanguage("ja"));

beforeEach(() => {
  vi.clearAllMocks();
  useRunEvents.setState({
    byConversation: {
      c1: {
        runId: "r1",
        timeline: { ...createTimeline("r1"), status: "running" },
      },
    },
  });
});

function renderQueue() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <SteeringQueue conversationId="c1" />
    </QueryClientProvider>,
  );
}

describe("SteeringQueue", () => {
  it("lists messages waiting for the next turn", async () => {
    renderQueue();
    expect(await screen.findByText("修正して")).toBeInTheDocument();
    expect(listSteering).toHaveBeenCalled();
  });

  it("rewrites a queued message before it is sent", async () => {
    renderQueue();
    await screen.findByText("修正して");
    fireEvent.click(screen.getByRole("button", { name: "指示を編集" }));
    const input = screen.getByRole("textbox", { name: "指示を編集" });
    fireEvent.change(input, { target: { value: "別の修正" } });
    fireEvent.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() =>
      expect(updateSteering).toHaveBeenCalledWith("r1", 1, "別の修正"),
    );
  });

  it("removes a queued message", async () => {
    renderQueue();
    await screen.findByText("修正して");
    fireEvent.click(screen.getByRole("button", { name: "指示を削除" }));
    await waitFor(() => expect(removeSteering).toHaveBeenCalledWith("r1", 1));
  });
});
