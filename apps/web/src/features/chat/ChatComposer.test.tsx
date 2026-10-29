import type { Row } from "@/app/api";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";

import { ChatComposer, type ChatComposerProps } from "./ChatComposer";
import type { Artifact, ChatModel, PermissionRule, Tool } from "./types";

const noop = () => {};

const model = (name: string): ChatModel =>
  ({
    id: "m1",
    created_at: "2026-01-01T00:00:00Z",
    data: { name, model_id: "test-model" },
  }) as ChatModel;

const props = (
  overrides: Partial<ChatComposerProps> = {},
): ChatComposerProps => ({
  text: "こんにちは",
  onTextChange: noop,
  onSubmit: noop,
  textareaRef: { current: null },
  attachments: [],
  onRemoveAttachment: noop,
  onAttach: noop,
  uploading: false,
  models: [model("Test Model")],
  agents: [],
  projects: [],
  tools: [],
  permissionRules: [],
  model: "m1",
  onModelChange: noop,
  mode: "chat",
  onModeChange: noop,
  researchMode: false,
  onResearchModeChange: noop,
  agent: "",
  onAgentChange: noop,
  selectedProject: "",
  onSelectedProjectChange: noop,
  temporaryMode: false,
  onTemporaryModeChange: noop,
  allowTools: false,
  onAllowToolsChange: noop,
  active: false,
  busy: false,
  needsProviderSetup: false,
  hasNoProviders: false,
  pageError: null,
  onRetry: noop,
  onConnectProvider: noop,
  onStop: noop,
  ...overrides,
});

const render = (overrides: Partial<ChatComposerProps> = {}) =>
  renderToStaticMarkup(
    <MemoryRouter>
      <ChatComposer {...props(overrides)} />
    </MemoryRouter>,
  );

describe("ChatComposer", () => {
  it("renders the textarea, send shortcut hint, model summary, and send button", () => {
    const html = render();
    expect(html).toContain('placeholder="MIX agent にメッセージを送信…"');
    expect(html).toContain("Enterで送信 / Shift+Enterで改行");
    expect(html).toContain("Test Model · chat");
    expect(html).toContain('aria-label="送信"');
    expect(html).toContain("MIX agent の回答には誤りが含まれることがあります");
    expect(html).not.toContain("temporary-notice");
  });

  it("shows temporary mode, attachments, permissions, and upload progress", () => {
    const attachments: Artifact[] = [
      {
        artifact_id: "img",
        name: "screenshot.png",
        mime: "image/png",
        size: 1024,
      },
      {
        artifact_id: "doc",
        name: "notes.txt",
        mime: "text/plain",
        size: 12,
      },
    ];
    const tools = [
      { id: "t1", default_permission: "deny" },
      { id: "t2", default_permission: "ask" },
      { id: "t3", default_permission: "deny" },
    ] as Tool[];
    const permissionRules = [
      {
        id: "p1",
        data: { agent_id: "", tool_id: "t1", permission: "allow" },
      },
    ] as PermissionRule[];
    const html = render({
      temporaryMode: true,
      allowTools: true,
      attachments,
      uploading: true,
      tools,
      permissionRules,
      pageError: new Error("送信に失敗しました"),
    });
    expect(html).toContain("一時モード");
    expect(html).toContain("許可中");
    expect(html).toContain("attachment-thumb");
    expect(html).toContain("screenshot.png");
    expect(html).toContain("notes.txt");
    expect(html).toContain("アップロード中…");
    expect(html).toContain("送信に失敗しました");
    expect(html).toContain("ツール権限: 許可 1");
    expect(html).toContain("確認 1");
    expect(html).toContain("拒否 1");
  });

  it("shows the stop control while a run is active", () => {
    const html = render({
      active: true,
      mode: "agent",
      researchMode: true,
      model: "auto",
    });
    expect(html).toContain('aria-label="停止"');
    expect(html).not.toContain('aria-label="送信"');
    expect(html).toContain("Auto · agent");
    expect(html).toContain("調査モード");
  });

  it("guides setup when providers or models are missing", () => {
    const providerHtml = render({
      needsProviderSetup: true,
      hasNoProviders: true,
    });
    expect(providerHtml).toContain("AI Providerが接続されていません");
    expect(providerHtml).toContain("Providerを接続");

    const modelHtml = render({
      needsProviderSetup: true,
      hasNoProviders: false,
    });
    expect(modelHtml).toContain("利用できるモデルがありません");
    expect(modelHtml).toContain("Providerを接続");
  });

  it("renders project and agent options plus the permission summary", () => {
    const html = render({
      projects: [
        {
          id: "proj",
          created_at: "2026-01-01T00:00:00Z",
          data: { name: "Research" },
        } as Row,
      ],
      agents: [
        {
          id: "agent",
          data: { name: "Researcher", mode: "agent" },
        },
      ] as ChatComposerProps["agents"],
      selectedProject: "proj",
      agent: "agent",
    });
    expect(html).toContain("Research");
    expect(html).toContain("Researcher");
    expect(html).toContain("ツール権限");
  });
});
