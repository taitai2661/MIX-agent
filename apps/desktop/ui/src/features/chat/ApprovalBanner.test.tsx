import { beforeAll, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { api } from "@/lib/api";
import { setLanguage } from "@/lib/i18n";
import { ApprovalBanner } from "./ApprovalBanner";
import type { ApprovalRequest } from "@/lib/runEvents";

beforeAll(() => setLanguage("ja"));

vi.mock("@/lib/api", () => ({
  api: {
    decideApproval: vi.fn().mockResolvedValue({ status: "ok" }),
  },
}));

describe("ApprovalBanner", () => {
  const approval: ApprovalRequest = {
    approvalId: "app-123",
    name: "terminal",
    arguments: { command: "npm test" },
  };

  it("renders tool name and required action buttons", () => {
    render(<ApprovalBanner approval={approval} />);
    expect(screen.getByText("terminal")).toBeInTheDocument();
    expect(screen.getByText("承認待ち:")).toBeInTheDocument();
    expect(screen.getByText("今回のみ許可")).toBeInTheDocument();
    expect(screen.getByText("拒否")).toBeInTheDocument();
  });

  it("calls decideApproval with grant: true when clicked approve", async () => {
    render(<ApprovalBanner approval={approval} />);
    const approveBtn = screen.getByText("今回のみ許可");
    fireEvent.click(approveBtn);
    expect(api.decideApproval).toHaveBeenCalledWith({
      id: "app-123",
      grant: true,
      remember: false,
    });
  });

  it("calls decideApproval with grant: false when clicked reject", async () => {
    render(<ApprovalBanner approval={approval} />);
    const rejectBtn = screen.getByText("拒否");
    fireEvent.click(rejectBtn);
    expect(api.decideApproval).toHaveBeenCalledWith({
      id: "app-123",
      grant: false,
      remember: false,
    });
  });
});
