import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/api";
import { TerminalPane } from "./TerminalPane";

vi.mock("@/lib/queries", () => ({
  useActiveRoot: () => "/tmp/workspace",
}));

vi.mock("@/lib/api", () => ({
  api: {
    startShellCommand: vi.fn(),
    pollShellCommand: vi.fn(),
    interruptShellCommand: vi.fn(),
    sendShellInput: vi.fn(),
  },
}));

const mocked = vi.mocked(api, true);

function poll(
  overrides: Partial<Awaited<ReturnType<typeof api.pollShellCommand>>>,
) {
  return {
    output: "",
    done: false,
    idle: false,
    persistent: true,
    exit_code: null,
    cwd: null,
    command: "echo hi",
    error: null,
    ...overrides,
  };
}

describe("TerminalPane", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocked.startShellCommand.mockResolvedValue({
      cwd: "/tmp/workspace",
      persistent: true,
    });
    mocked.interruptShellCommand.mockResolvedValue(undefined);
    mocked.sendShellInput.mockResolvedValue({
      ok: true,
      content: {},
      error: null,
    });
  });

  it("streams output while the command runs and keeps the exit code", async () => {
    mocked.pollShellCommand
      .mockResolvedValueOnce(poll({ command: "echo hi" }))
      .mockResolvedValueOnce(poll({ output: "hi\n" }))
      .mockResolvedValueOnce(
        poll({ output: "hi\n", done: true, exit_code: 0 }),
      );

    render(<TerminalPane />);
    const input = screen.getByLabelText(/実行するコマンド|Command to run/);
    fireEvent.change(input, { target: { value: "echo hi" } });
    fireEvent.submit(input.closest("form")!);

    await waitFor(() =>
      expect(mocked.startShellCommand).toHaveBeenCalledWith({
        root: "/tmp/workspace",
        command: "echo hi",
        max_wall_clock_ms: 600_000,
      }),
    );
    await waitFor(() =>
      expect(screen.getByText("hi", { exact: false })).toBeInTheDocument(),
    );
    await waitFor(() => expect(screen.getByText("0")).toBeInTheDocument());
  });

  it("sends input to a running command instead of starting another", async () => {
    // The first poll never completes, so the pane stays in input mode.
    mocked.pollShellCommand.mockResolvedValue(poll({ command: "cat" }));

    render(<TerminalPane />);
    const input = screen.getByLabelText(/実行するコマンド|Command to run/);
    fireEvent.change(input, { target: { value: "cat" } });
    fireEvent.submit(input.closest("form")!);

    await waitFor(() =>
      expect(
        screen.getByLabelText(
          /実行中のコマンドへの入力|Input for the running command/,
        ),
      ).toBeInTheDocument(),
    );
    fireEvent.change(
      screen.getByLabelText(
        /実行中のコマンドへの入力|Input for the running command/,
      ),
      {
        target: { value: "hello" },
      },
    );
    fireEvent.submit(
      screen
        .getByLabelText(
          /実行中のコマンドへの入力|Input for the running command/,
        )
        .closest("form")!,
    );

    await waitFor(() =>
      expect(mocked.sendShellInput).toHaveBeenCalledWith({
        root: "/tmp/workspace",
        input: "hello",
        wait_ms: 0,
      }),
    );
    expect(mocked.startShellCommand).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: /停止|Stop/ }));
    await waitFor(() =>
      expect(mocked.interruptShellCommand).toHaveBeenCalledWith(
        "/tmp/workspace",
      ),
    );
  });
});
