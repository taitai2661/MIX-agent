import { describe, expect, it } from "vitest";

import { permissionInfo, resolvedPermission, riskLabel } from "./Tools";

describe("Tools permission presentation", () => {
  const tool = { id: "write_file", default_permission: "ask" };

  it("uses the selected Agent's latest rule before the default permission", () => {
    const rules = [
      { data: { agent_id: "agent-a", tool_id: "write_file", permission: "deny" } },
      { data: { agent_id: "agent-b", tool_id: "write_file", permission: "allow" } },
    ];
    expect(resolvedPermission(tool, rules, "agent-a")).toBe("deny");
    expect(resolvedPermission(tool, rules, "agent-b")).toBe("allow");
    expect(resolvedPermission(tool, rules, "")).toBe("ask");
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
