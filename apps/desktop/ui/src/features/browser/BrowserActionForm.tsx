import { useCallback, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Camera,
  ChevronRight,
  Code,
  FileText,
  Globe,
  Keyboard,
  MousePointerClick,
  Play,
  RotateCw,
  ScrollText,
  Search,
  Square,
  Terminal,
  Type as TypeIcon,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { optString } from "@/lib/runEvents";
import { Card, Field, Row } from "@/components/layout";
import { Button } from "@/components/ui/button";
import { Input, Select, Textarea } from "@/components/ui/input";
import type { LogEntry } from "./browserLog";

type Action =
  | "info"
  | "navigate"
  | "dom"
  | "extract"
  | "snapshot"
  | "screenshot"
  | "evaluate"
  | "click"
  | "type"
  | "press"
  | "hover"
  | "select"
  | "check"
  | "scroll"
  | "wait_for"
  | "tabs"
  | "reload"
  | "back"
  | "forward";

const ACTIONS: Array<{
  id: Action;
  label: string;
  icon: typeof Globe;
  hint: string;
}> = [
  {
    id: "info",
    label: "Page info",
    icon: FileText,
    hint: "Title, URL and viewport",
  },
  { id: "navigate", label: "Navigate", icon: Globe, hint: "Load a URL" },
  {
    id: "snapshot",
    label: "Snapshot",
    icon: Code,
    hint: "Read an accessibility tree and get stable refs",
  },
  {
    id: "dom",
    label: "DOM",
    icon: Code,
    hint: "innerHTML of a selector (or the body)",
  },
  {
    id: "extract",
    label: "Extract",
    icon: Search,
    hint: "Structured text and links from matching elements",
  },
  {
    id: "click",
    label: "Click",
    icon: MousePointerClick,
    hint: "Click a CSS selector or a snapshot ref",
  },
  {
    id: "type",
    label: "Type",
    icon: TypeIcon,
    hint: "Focus and type with real keyboard events",
  },
  {
    id: "press",
    label: "Press",
    icon: Keyboard,
    hint: "Send Enter, Tab, Escape, arrows, ...",
  },
  {
    id: "hover",
    label: "Hover",
    icon: MousePointerClick,
    hint: "Move the mouse over an element",
  },
  {
    id: "select",
    label: "Select",
    icon: ChevronRight,
    hint: "Choose an option in a native select",
  },
  {
    id: "check",
    label: "Check",
    icon: Square,
    hint: "Set a checkbox or radio state",
  },
  { id: "scroll", label: "Scroll", icon: ScrollText, hint: "Scroll by dx/dy" },
  {
    id: "wait_for",
    label: "Wait",
    icon: Search,
    hint: "Wait for selector, URL, text, load or JS",
  },
  {
    id: "screenshot",
    label: "Screenshot",
    icon: Camera,
    hint: "Capture the viewport",
  },
  { id: "evaluate", label: "Evaluate", icon: Terminal, hint: "Run JavaScript" },
  { id: "tabs", label: "Tabs", icon: Play, hint: "List open tabs" },
  { id: "reload", label: "Reload", icon: RotateCw, hint: "Reload the page" },
  { id: "back", label: "Back", icon: ArrowLeft, hint: "Go back in history" },
  {
    id: "forward",
    label: "Forward",
    icon: ArrowRight,
    hint: "Go forward in history",
  },
];

interface Props {
  workspaceId: string;
  onResult: (entry: Omit<LogEntry, "id">) => void;
}

/** Manual single-action form. State lives here so the pane only composes. */
export function BrowserActionForm({ workspaceId, onResult }: Props) {
  const [action, setAction] = useState<Action>("info");
  const [url, setUrl] = useState("");
  const [selector, setSelector] = useState("");
  const [reference, setReference] = useState("");
  const [text, setText] = useState("");
  const [key, setKey] = useState("Enter");
  const [value, setValue] = useState("");
  const [checked, setChecked] = useState(true);
  const [clear, setClear] = useState(false);
  const [script, setScript] = useState("");
  const [scrollY, setScrollY] = useState(600);
  const [condition, setCondition] = useState("selector");
  const [busy, setBusy] = useState(false);

  const run = useCallback(async () => {
    setBusy(true);
    try {
      const scrollValue = Number(scrollY);
      const out = await api.browserInvoke({
        workspace_id: workspaceId,
        action,
        url: url || null,
        selector: selector || null,
        ref: reference || null,
        text: text || null,
        key,
        value: value || null,
        checked,
        clear,
        x: null,
        y:
          action === "scroll" && Number.isFinite(scrollValue)
            ? scrollValue
            : null,
        script: script || null,
        full_page: false,
        condition: action === "wait_for" ? condition : null,
        timeout_ms: 10_000,
      });
      // Shared `optString` guard (see `lib/runEvents.ts`) instead of an
      // inline `typeof` check on the `unknown` payload.
      const detail =
        optString(out.payload) ?? JSON.stringify(out.payload, null, 2);
      onResult({
        action,
        ok: out.ok,
        summary: out.ok ? "ok" : (out.error ?? "failed"),
        detail: detail.length > 4000 ? `${detail.slice(0, 4000)}\n…` : detail,
        artifactId: out.artifact_id,
      });
    } catch (e) {
      onResult({ action, ok: false, summary: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  }, [
    workspaceId,
    action,
    url,
    selector,
    reference,
    text,
    key,
    value,
    checked,
    clear,
    script,
    scrollY,
    condition,
    onResult,
  ]);

  const active = ACTIONS.find((a) => a.id === action)!;
  // Gate each field on the actions that actually use it. Showing a selector
  // box for `screenshot` was the old bug; showing every new field for every
  // action would repeat it at a larger scale.
  const needsSelector = [
    "dom",
    "extract",
    "click",
    "type",
    "hover",
    "select",
    "check",
    "wait_for",
  ].includes(action);
  const needsRef = ["click", "type", "hover", "select", "check"].includes(
    action,
  );
  const needsText = action === "type";

  return (
    <Card title={t("操作")}>
      <Row className="gap-1.5">
        {ACTIONS.map((a) => {
          const Icon = a.icon;
          return (
            <Button
              key={a.id}
              size="sm"
              selected={a.id === action}
              onClick={() => setAction(a.id)}
              title={t(a.hint)}
            >
              <Icon size={12} /> {t(a.label)}
            </Button>
          );
        })}
      </Row>
      <p className="m-0 text-xs text-muted">{t(active.hint)}</p>

      {action === "navigate" && (
        <Field>
          <Input
            aria-label={t("開くURL")}
            placeholder="https://example.com"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
          />
        </Field>
      )}
      {needsSelector && (action !== "wait_for" || condition === "selector") && (
        <Field>
          <Input
            aria-label={t("CSSセレクター（例: #search、body）")}
            placeholder={t("CSSセレクター（例: #search、body）")}
            value={selector}
            onChange={(e) => setSelector(e.target.value)}
          />
        </Field>
      )}
      {needsRef && (
        <Field>
          <Input
            aria-label={t("スナップショットの ref（例: r12）")}
            placeholder={t("スナップショットの ref（任意・selectorより優先）")}
            value={reference}
            onChange={(e) => setReference(e.target.value)}
          />
        </Field>
      )}
      {needsText && (
        <>
          <Field>
            <Input
              aria-label={t("入力する文字")}
              placeholder={t("入力する文字")}
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
          </Field>
          <label className="flex items-center gap-2 text-xs text-muted">
            <input
              type="checkbox"
              checked={clear}
              onChange={(e) => setClear(e.target.checked)}
            />
            {t("入力前にフィールドを空にする")}
          </label>
        </>
      )}
      {action === "press" && (
        <Field>
          <Input
            aria-label={t("送信するキー")}
            placeholder="Enter"
            value={key}
            onChange={(e) => setKey(e.target.value)}
          />
        </Field>
      )}
      {action === "select" && (
        <Field>
          <Input
            aria-label={t("選択する値")}
            placeholder={t("option の value")}
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
        </Field>
      )}
      {action === "check" && (
        <label className="flex items-center gap-2 text-xs text-muted">
          <input
            type="checkbox"
            checked={checked}
            onChange={(e) => setChecked(e.target.checked)}
          />
          {t("チェックする")}
        </label>
      )}
      {action === "wait_for" && (
        <Row className="gap-1.5">
          <Select
            aria-label={t("待機条件")}
            value={condition}
            onChange={(e) => setCondition(e.target.value)}
          >
            <option value="selector">selector</option>
            <option value="url">url</option>
            <option value="text">text</option>
            <option value="load">load</option>
            <option value="function">function</option>
          </Select>
          {condition === "url" && (
            <Input
              className="min-w-0 flex-1"
              aria-label={t("URLに含まれるまで待機…")}
              placeholder={t("URLに含まれるまで待機…")}
              value={url}
              onChange={(e) => setUrl(e.target.value)}
            />
          )}
          {condition === "text" && (
            <Input
              className="min-w-0 flex-1"
              aria-label={t("本文に含まれるまで待機…")}
              placeholder={t("本文に含まれるまで待機…")}
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
          )}
          {condition === "function" && (
            <Input
              className="min-w-0 flex-1 font-mono"
              aria-label={t("JavaScript 式")}
              placeholder="() => document.readyState === 'complete'"
              value={script}
              onChange={(e) => setScript(e.target.value)}
            />
          )}
        </Row>
      )}
      {action === "scroll" && (
        <Field>
          <Input
            type="number"
            aria-label={t("縦スクロール量")}
            placeholder="scroll dy"
            value={Number.isFinite(scrollY) ? scrollY : ""}
            onChange={(e) => {
              const raw = e.target.value;
              if (raw === "") {
                setScrollY(0);
                return;
              }
              const next = Number(raw);
              setScrollY(Number.isFinite(next) ? next : 0);
            }}
          />
        </Field>
      )}
      {action === "evaluate" && (
        <Field>
          <Textarea
            className="font-mono"
            rows={4}
            aria-label={t("実行する JavaScript")}
            placeholder="document.title"
            value={script}
            onChange={(e) => setScript(e.target.value)}
          />
        </Field>
      )}

      <Row>
        <Button variant="primary" disabled={busy} onClick={() => void run()}>
          <ChevronRight size={12} /> {busy ? t("実行中…") : t("実行")}
        </Button>
      </Row>
    </Card>
  );
}
