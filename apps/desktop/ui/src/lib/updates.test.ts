import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/api";
import { downloadPendingUpdate, useUpdateStore } from "./updates";
import type { UpdateInfo } from "./types";

vi.mock("@/lib/api", () => ({
  api: {
    downloadUpdate: vi.fn(),
  },
}));

const mockedApi = vi.mocked(api);

function info(version: string): UpdateInfo {
  return {
    current_version: "0.5.6",
    version,
    notes: null,
    date: null,
    installable: true,
  };
}

describe("downloadPendingUpdate", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUpdateStore.setState({
      phase: "idle",
      pending: null,
      progress: null,
      error: null,
      hydrated: true,
    });
  });

  it("stages the payload instead of reporting an install", async () => {
    mockedApi.downloadUpdate.mockResolvedValue(info("0.5.7"));

    await downloadPendingUpdate();

    const state = useUpdateStore.getState();
    expect(state.phase).toBe("ready");
    expect(state.pending?.version).toBe("0.5.7");
    expect(state.error).toBeNull();
  });

  it("does not start a second download while one is in flight", async () => {
    let resolve!: (value: UpdateInfo) => void;
    mockedApi.downloadUpdate.mockReturnValue(
      new Promise<UpdateInfo>((done) => {
        resolve = done;
      }),
    );

    const first = downloadPendingUpdate();
    const second = downloadPendingUpdate();
    await second;

    expect(mockedApi.downloadUpdate).toHaveBeenCalledTimes(1);

    resolve(info("0.5.7"));
    await first;
    expect(useUpdateStore.getState().phase).toBe("ready");
  });

  it("keeps the error visible so the manual retry button can be used", async () => {
    mockedApi.downloadUpdate.mockRejectedValue("network down");

    await downloadPendingUpdate();

    const state = useUpdateStore.getState();
    expect(state.phase).toBe("error");
    expect(state.error).toBe("network down");
    expect(state.pending).toBeNull();
  });
});
