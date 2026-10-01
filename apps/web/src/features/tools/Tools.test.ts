import { describe, expect, it } from "vitest";

import { permissionInfo, resolvedPermission, riskLabel, toolCategories, toolCategory } from "./Tools";

describe("Tools permission presentation", () => {
  const tool = { id: "write_file", default_permission: "allow" };

  it("uses the selected Agent's latest rule before the default permission", () => {
    const rules = [
      { data: { agent_id: "agent-a", tool_id: "write_file", permission: "deny" } },
      { data: { agent_id: "agent-b", tool_id: "write_file", permission: "allow" } },
    ];
    expect(resolvedPermission(tool, rules, "agent-a")).toBe("deny");
    expect(resolvedPermission(tool, rules, "agent-b")).toBe("allow");
    expect(resolvedPermission(tool, rules, "")).toBe("allow");
  });

  it("defaults to allow when a tool carries no permission at all", () => {
    expect(resolvedPermission({ id: "run_terminal" }, undefined, "")).toBe("allow");
  });

  it("keeps the three permission choices understandable in Japanese", () => {
    expect(permissionInfo.allow.label).toBe("常に許可");
    expect(permissionInfo.ask.description).toContain("確認");
    expect(permissionInfo.deny.description).toContain("実行できません");
  });

  it("translates existing risk values without changing them", () => {
    expect(riskLabel("read")).toBe("読み取り中心");
    expect(riskLabel("write")).toBe("変更を伴う");
    expect(riskLabel("external")).toBe("外部サービス");
  });
});

describe("Tool category grouping", () => {
  const tool = (id: string, extra: any = {}) => ({ id, source: "builtin", ...extra });

  it("groups built-in tools by their capability area", () => {
    expect(toolCategory(tool("web_search"))).toBe("web");
    expect(toolCategory(tool("web_fetch_pdf"))).toBe("web");
    expect(toolCategory(tool("write_file"))).toBe("files");
    expect(toolCategory(tool("run_terminal"))).toBe("terminal");
    expect(toolCategory(tool("process_stop"))).toBe("terminal");
    expect(toolCategory(tool("browser_open"))).toBe("browser");
    expect(toolCategory(tool("knowledge_add"))).toBe("knowledge");
    expect(toolCategory(tool("web_clip_save"))).toBe("knowledge");
    expect(toolCategory(tool("memory_search"))).toBe("memory");
    expect(toolCategory(tool("skill_add"))).toBe("skill");
    expect(toolCategory(tool("update_plan"))).toBe("plan");
    expect(toolCategory(tool("schedule_create"))).toBe("schedule");
  });

  it("routes MCP and custom tools to their own groups", () => {
    expect(toolCategory(tool("mcp_github_issue", { source: "mcp" }))).toBe("mcp");
    expect(toolCategory(tool("custom_api", { source: "custom" }))).toBe("custom");
  });

  it("provides a stable category catalog with unique keys", () => {
    const keys = toolCategories.map((category) => category.key);
    expect(new Set(keys).size).toBe(keys.length);
    expect(keys).toContain("web");
    expect(keys).toContain("mcp");
  });
});