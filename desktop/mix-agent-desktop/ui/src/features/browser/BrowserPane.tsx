import { useCallback, useEffect, useState } from "react";
import {
  Camera, ChevronRight, Code, FileText, Globe, MousePointerClick, RefreshCw,
  ScrollText, Search, Terminal, Type as TypeIcon,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { useUi } from "@/lib/ui";
import { Card, Field, Row, Section } from "@/components/layout";
import { EmptyState } from "@/components/shared";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useConfirm } from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";
import { Input, Select, Textarea } from "@/components/ui/input";
import type { BrowserProfileView, BrowserStatus } from "@/lib/types";

type Action =
  | "info" | "navigate" | "dom" | "screenshot" | "evaluate"
  | "click" | "type" | "scroll" | "wait_for";

const ACTIONS: Array<{ id: Action; label: string; icon: typeof Globe; hint: string }> = [
  { id: "info", label: "Page info", icon: FileText, hint: "Title, URL and viewport" },
  { id: "navigate", label: "Navigate", icon: Globe, hint: "Load a URL" },
  { id: "dom", label: "DOM", icon: Code, hint: "innerHTML of a selector (or the body)" },
  { id: "click", label: "Click", icon: MousePointerClick, hint: "Click a CSS selector" },
  { id: "type", label: "Type", icon: TypeIcon, hint: "Type text into a selector" },
  { id: "scroll", label: "Scroll", icon: ScrollText, hint: "Scroll by dx/dy" },
  { id: "wait_for", label: "Wait", icon: Search, hint: "Wait for selector, URL or JS" },
  { id: "screenshot", label: "Screenshot", icon: Camera, hint: "Capture the viewport" },
  { id: "evaluate", label: "Evaluate", icon: Terminal, hint: "Run JavaScript" },
];

interface LogEntry {
  id: number;
  action: Action;
  ok: boolean;
  summary: string;
  detail?: string;
  artifactId?: string | null;
}

let nextId = 0;

export function BrowserPane() {
  const [status, setStatus] = useState<BrowserStatus | null>(null);
  const [profiles, setProfiles] = useState<BrowserProfileView[]>([]);
  // Default to the active conversation's workspace so a manual sign-in lands in
  // the profile the agent will later browse with.
  const activeWorkspace = useUi((s) => s.workspaceId);
  const [workspaceId, setWorkspaceId] = useState<string>(
    () => activeWorkspace ?? crypto.randomUUID(),
  );
  const [action, setAction] = useState<Action>("info");
  const [url, setUrl] = useState("");
  const [selector, setSelector] = useState("");
  const [text, setText] = useState("");
  const [script, setScript] = useState("");
  const [scrollY, setScrollY] = useState(600);
  const [condition, setCondition] = useState("selector");
  const [busy, setBusy] = useState(false);
  // `null` until the first status read lands. The pane used to show a literal
  // "…" inside a badge, which read as a status value rather than as "loading".
  const [loading, setLoading] = useState(true);
  const [log, setLog] = useState<LogEntry[]>([]);
  const { confirm, dialog } = useConfirm();

  const refresh = useCallback(async () => {
    try {
      const [s, p] = await Promise.all([api.browserStatus(), api.browserProfiles()]);
      setStatus(s);
      setProfiles(p);
      if (p.length > 0 && !p.some((x) => x.workspace_id === workspaceId)) {
        const preferred =
          activeWorkspace && p.some((x) => x.workspace_id === activeWorkspace)
            ? activeWorkspace
            : p[0].workspace_id;
        setWorkspaceId(preferred);
      }
    } catch (e) {
      setLog((l) => [...l, { id: nextId++, action: "info", ok: false, summary: String(e) }]);
    } finally {
      setLoading(false);
    }
  }, [workspaceId, activeWorkspace]);

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const run = useCallback(async () => {
    setBusy(true);
    try {
      const out = await api.browserInvoke({
        workspace_id: workspaceId,
        action,
        url: url || null,
        selector: selector || null,
        text: text || null,
        x: null,
        y: action === "scroll" ? scrollY : null,
        script: script || null,
        full_page: false,
        condition: action === "wait_for" ? condition : null,
        timeout_ms: 10_000,
      });
      const detail = typeof out.payload === "string"
        ? out.payload
        : JSON.stringify(out.payload, null, 2);
      setLog((l) => [
        {
          id: nextId++,
          action,
          ok: out.ok,
          summary: out.ok ? "ok" : (out.error ?? "failed"),
          detail: detail.length > 4000 ? `${detail.slice(0, 4000)}\n…` : detail,
          artifactId: out.artifact_id,
        },
        ...l,
      ].slice(0, 50));
    } catch (e) {
      setLog((l) => [{ id: nextId++, action, ok: false, summary: String(e) }, ...l].slice(0, 50));
    } finally {
      setBusy(false);
    }
  }, [workspaceId, action, url, selector, text, script, scrollY, condition]);

  const active = ACTIONS.find((a) => a.id === action)!;
  const needs = (f: "url" | "selector" | "text" | "script") => f;
  const knownProfile = profiles.some((p) => p.workspace_id === workspaceId);

  return (
    <Section>
      <Card title={t("Backend")} icon={<Globe size={13} />} action={
        <Button size="sm" onClick={() => void refresh()} title={t("Refresh status")}>
          <RefreshCw size={12} /> {t("Refresh")}
        </Button>
      }>
        <Row className="gap-1.5">
          <Badge size="sm" className="font-mono">
            {status?.backend ?? (loading ? t("読み込み中…") : "—")}
          </Badge>
          <Badge size="sm">{status?.headless ? t("headless") : t("headed")}</Badge>
          <Badge size="sm">{status?.profiles ?? 0} {t("profiles")}</Badge>
          <Badge size="sm">{status?.artifacts ?? 0} {t("artifacts")}</Badge>
        </Row>
        {status?.executable ? (
          <code className="break-all text-xs text-faint">{status.executable}</code>
        ) : loading ? (
          <p className="m-0 inline-flex items-center gap-1.5 text-xs text-muted">
            <Spinner size={11} /> {t("読み込み中…")}
          </p>
        ) : (
          <p className="m-0 text-xs text-muted">
            {t("No Chromium binary resolved. Install a system Chromium or bundle one to enable the Browser.")}
          </p>
        )}
      </Card>

      <Card title={t("Workspace profile")}>
        <Row className="gap-2">
          <Select
            value={knownProfile ? workspaceId : "new"}
            onChange={(e) => setWorkspaceId(e.target.value === "new" ? crypto.randomUUID() : e.target.value)}
          >
            {!knownProfile && <option value="new">{t("new profile")}</option>}
            {profiles.map((p) => (
              <option key={p.workspace_id} value={p.workspace_id}>
                {p.display_name} {p.provisioned ? "" : t("(not provisioned)")}
              </option>
            ))}
          </Select>
          <code className="min-w-0 flex-1 truncate font-mono text-xs text-faint">{workspaceId}</code>
          <Button
            disabled={busy}
            onClick={async () => {
              // Wiping a profile's cookies, storage and cache is not reversible,
              // so it is a `danger` confirmation like every other destructive
              // action rather than the native alert it used to be.
              const ok = await confirm({
                title: t("プロファイルをリセットしますか？"),
                description: t("Cookie・localStorage・キャッシュが消えます。"),
                confirmLabel: t("リセット"),
                tone: "danger",
              });
              if (!ok) return;
              try {
                await api.browserResetProfile(workspaceId);
                setWorkspaceId(crypto.randomUUID());
                await refresh();
              } catch (e) {
                setLog((l) => [{ id: nextId++, action: "info", ok: false, summary: String(e) }, ...l]);
              }
            }}
          >
            {t("Reset")}
          </Button>
        </Row>
      </Card>

      <Card title={t("Action")}>
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
                <Icon size={11} /> {t(a.label)}
              </Button>
            );
          })}
        </Row>
        <p className="m-0 text-xs text-muted">{t(active.hint)}</p>

        {needs("url") && action === "navigate" && (
          <Field>
            <Input
              placeholder="https://example.com"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
            />
          </Field>
        )}
        {needs("selector") && (
          <Field>
            <Input
              placeholder={t("CSS selector, e.g. #search or body")}
              value={selector}
              onChange={(e) => setSelector(e.target.value)}
            />
          </Field>
        )}
        {needs("text") && (
          <Field>
            <Input
              placeholder={t("text to type")}
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
          </Field>
        )}
        {action === "wait_for" && (
          <Row className="gap-1.5">
            <Select value={condition} onChange={(e) => setCondition(e.target.value)}>
              <option value="selector">selector</option>
              <option value="url">url</option>
              <option value="function">function</option>
            </Select>
            {condition === "url" && (
              <Input
                className="min-w-0 flex-1"
                placeholder={t("wait until URL contains…")}
                value={url}
                onChange={(e) => setUrl(e.target.value)}
              />
            )}
            {condition === "function" && (
              <Input
                className="min-w-0 flex-1 font-mono"
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
              placeholder="scroll dy"
              value={scrollY}
              onChange={(e) => setScrollY(Number(e.target.value))}
            />
          </Field>
        )}
        {action === "evaluate" && (
          <Field>
            <Textarea
              className="font-mono"
              rows={4}
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

      <Card title={t("Activity")}>
        {log.length === 0 ? (
          <EmptyState title={t("No actions yet.")} />
        ) : (
          <ul className="flex flex-col">
            {log.map((e) => (
              <li key={e.id} className="border-t border-line py-2 first:border-0 first:pt-0">
                <Row className="gap-2">
                  <Badge size="sm" className="font-mono">{e.action}</Badge>
                  <span className={cn("text-xs", e.ok ? "text-success" : "text-danger")}>
                    {e.ok ? t("ok") : t("error")}: {e.summary}
                  </span>
                  {e.artifactId && (
                    <Badge size="sm" className="font-mono">
                      artifact {e.artifactId.slice(0, 8)}
                    </Badge>
                  )}
                </Row>
                {e.detail && (
                  <details className="group">
                    <summary className="mt-1 flex cursor-pointer list-none items-center gap-1 text-xs text-muted transition-colors hover:text-fg">
                      <ChevronRight
                        size={11}
                        aria-hidden="true"
                        className="shrink-0 transition-transform group-open:rotate-90"
                      />
                      {t("output")}
                    </summary>
                    <pre className="mt-1.5 max-h-64 overflow-auto whitespace-pre-wrap break-words font-mono text-xs leading-4 text-muted">
                      {e.detail}
                    </pre>
                  </details>
                )}
              </li>
            ))}
          </ul>
        )}
      </Card>

      {profiles.length > 0 && (
        <Card title={t("Known profiles")}>
          <ul className="flex flex-col gap-1 text-xs">
            {profiles.map((p) => (
              <li key={p.workspace_id} className="flex items-center gap-2">
                <span className="text-muted">{p.display_name}</span>
                <code className="font-mono text-xs text-faint">{p.workspace_id.slice(0, 8)}</code>
                <Badge tone={p.provisioned ? "success" : "neutral"} size="sm">
                  {p.provisioned ? t("ready") : t("empty")}
                </Badge>
              </li>
            ))}
          </ul>
          <p className="m-0 text-xs text-muted">
            {t("Each workspace keeps its own cookies, localStorage and cache, matching the agent's workspace.")}
          </p>
        </Card>
      )}
      {dialog}
    </Section>
  );
}
