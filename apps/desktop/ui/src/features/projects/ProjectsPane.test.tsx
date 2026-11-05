import { beforeAll, describe, expect, it, vi } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { setLanguage } from "@/lib/i18n";
import { api } from "@/lib/api";
import { ProjectsPane } from "./ProjectsPane";

vi.mock("@/lib/api", () => ({
  api: {
    listProjects: vi.fn(async () => []),
    listConversationsForProject: vi.fn(async () => ({
      items: [],
      has_more: false,
    })),
    createProject: vi.fn(async () => ({
      id: "p1",
      name: "demo",
      instructions: "",
      rootPath: "/tmp/demo",
      additionalPaths: [],
      sources: [],
      verifyCommand: null,
      formatCommand: null,
      verifyCandidates: [],
      created_at: "2026-01-01T00:00:00Z",
      updated_at: "2026-01-01T00:00:00Z",
    })),
    updateProject: vi.fn(async () => {
      throw new Error("not under test");
    }),
    deleteProject: vi.fn(async () => undefined),
    pickFolder: vi.fn(async () => "/tmp/demo"),
  },
}));

const mockedApi = vi.mocked(api);

// jsdom reports en-US; pin the language so labels are deterministic.
beforeAll(() => setLanguage("ja"));

function renderPane() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ProjectsPane />
    </QueryClientProvider>,
  );
}

describe("ProjectsPane creation", () => {
  it("cannot create a project without a folder", async () => {
    const user = userEvent.setup();
    renderPane();

    const create = screen.getByRole("button", { name: "作成" });
    expect(create).toBeDisabled();

    await user.type(screen.getByPlaceholderText("Project 名"), "demo");
    expect(create).toBeDisabled();
    expect(mockedApi.createProject).not.toHaveBeenCalled();
  });

  it("does not create twice when Enter is pressed during a pending create", async () => {
    const user = userEvent.setup();
    const original = mockedApi.createProject.getMockImplementation();
    let resolveCreate: (() => void) | undefined;
    mockedApi.createProject.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveCreate = () =>
            resolve({
              id: "p1",
              name: "demo",
              instructions: "",
              rootPath: "/tmp/demo",
              additionalPaths: [],
              sources: [],
              verifyCommand: null,
              formatCommand: null,
              verifyCandidates: [],
              created_at: "2026-01-01T00:00:00Z",
              updated_at: "2026-01-01T00:00:00Z",
            });
        }),
    );
    renderPane();

    await user.click(screen.getByRole("button", { name: "フォルダを選択" }));
    await screen.findByText("/tmp/demo");
    const name = screen.getByPlaceholderText("Project 名");
    await user.type(name, "demo");
    await user.keyboard("{Enter}");
    await waitFor(() =>
      expect(mockedApi.createProject).toHaveBeenCalledTimes(1),
    );

    // The Create button is disabled while pending; Enter used to bypass it.
    await user.keyboard("{Enter}");
    expect(mockedApi.createProject).toHaveBeenCalledTimes(1);

    await act(async () => {
      resolveCreate?.();
    });
    if (original) mockedApi.createProject.mockImplementation(original);
  });

  it("creates a project rooted at the picked folder", async () => {
    const user = userEvent.setup();
    renderPane();

    await user.click(screen.getByRole("button", { name: "フォルダを選択" }));
    expect(mockedApi.pickFolder).toHaveBeenCalled();
    await waitFor(() =>
      expect(screen.getByText("/tmp/demo")).toBeInTheDocument(),
    );

    const create = screen.getByRole("button", { name: "作成" });
    await waitFor(() => expect(create).toBeEnabled());

    await user.type(screen.getByPlaceholderText("Project 名"), "demo");
    await user.click(create);

    await waitFor(() =>
      expect(mockedApi.createProject).toHaveBeenCalledWith({
        name: "demo",
        instructions: "",
        rootPath: "/tmp/demo",
        sources: [],
      }),
    );
  });
});
