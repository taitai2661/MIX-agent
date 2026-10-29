import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  Providers,
  visibleProviderFields,
  type ProviderPreset,
} from "./Providers";

const openai: ProviderPreset = {
  id: "openai",
  name: "OpenAI",
  category: "直接接続",
  kind: "openai",
  default_url: "https://api.openai.com/v1",
  api_key_required: true,
  allow_private_default: false,
  extra_config_schema: [],
};

const ollama: ProviderPreset = {
  id: "ollama",
  name: "Ollama",
  category: "ローカル",
  kind: "ollama",
  default_url: "http://host.docker.internal:11434/v1",
  api_key_required: false,
  allow_private_default: true,
  extra_config_schema: [],
};

const custom: ProviderPreset = {
  id: "custom",
  name: "カスタム",
  category: "カスタム",
  kind: "compatible",
  default_url: "",
  api_key_required: true,
  allow_private_default: false,
  extra_config_schema: [],
};

it("hides connection details for a known cloud provider", () => {
  expect(visibleProviderFields(openai, true)).toEqual({
    picker: false,
    name: false,
    kind: false,
    url: false,
    apiKey: true,
    extra: false,
  });
});

it("skips the API key for local providers", () => {
  expect(visibleProviderFields(ollama, true).apiKey).toBe(false);
});

it("asks for name, URL, and connection type on custom providers", () => {
  expect(visibleProviderFields(custom, true)).toMatchObject({
    name: true,
    kind: true,
    url: true,
    apiKey: true,
  });
});

it("shows popular connections instead of the advanced form", () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  client.setQueryData(["/providers"], []);
  client.setQueryData(["/provider-presets"], [openai, ollama, custom]);
  const html = renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <Providers />
    </QueryClientProvider>,
  );
  expect(html).toContain("よく使う接続");
  expect(html).toContain("OpenAI");
  expect(html).toContain("Ollama");
  expect(html).not.toContain("カスタム接続方式");
  expect(html).not.toContain("従来の接続設定（編集時のみ）");
});
