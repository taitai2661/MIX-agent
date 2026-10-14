import { useCallback, useEffect, useState } from "react";
import {
  Camera, ChevronRight, Code, FileText, Globe, MousePointerClick, RefreshCw,
  ScrollText, Search, Terminal, Type as TypeIcon,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { optString } from "@/lib/runEvents";
import { useUi } from "@/lib/ui";
import { Card, Field, Row, Section } from "@/components/layout";
import { EmptyState, payloadBlock } from "@/components/shared";
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

  // Keep the manual-sign-in profile on the active conversation's workspace.
  // The initial state captures it once; without this the pane goes stale
  // after switching threads.
  useEffect(() => {
    if (activeWorkspace) setWorkspaceId(activeWorkspace);
  }, [activeWorkspace]);

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
      setLog((l) => [...l, { id: nextId++, action: "info", ok: false, summary: errorMessage(e) }]);
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
      const scrollValue = Number(scrollY);
      const out = await api.browserInvoke({
        workspace_id: workspaceId,
        action,
        url: url || null,
        selector: selector || null,
        text: text || null,
        x: null,
        y: action === "scroll" && Number.isFinite(scrollValue) ? scrollValue : null,
        script: script || null,
        full_page: false,
        condition: action === "wait_for" ? condition : null,
        timeout_ms: 10_000,
      });
      // Shared `optString` guard (see `lib/runEvents.ts`) instead of an
      // inline `typeof` check on the `unknown` payload.
      const detail = optString(out.payload) ?? JSON.stringify(out.payload, null, 2);
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
      setLog((l) => [{ id: nextId++, action, ok: false, summary: errorMessage(e) }, ...l].slice(0, 50));
    } finally {
      setBusy(false);
    }
  }, [workspaceId, action, url, selector, text, script, scrollY, condition]);

  const active = ACTIONS.find((a) => a.id === action)!;
  // The old `needs` returned its argument (always truthy), so every action
  // rendered the selector and text inputs — a URL-less screenshot form with
  // two fields that do nothing. Gate each field on the actions that use it.
  const needsSelector = ["dom", "click", "type", "wait_for"].includes(action);
  const needsText = action === "type";
  const knownProfile = profiles.some((p) => p.workspace_id === workspaceId);

  return (
    <Section>
      <Card title={t("バックエンド")} icon={<Globe size={13} />} action={
        <Button variant="ghost" size="sm" onClick={() => void refresh()} title={t("状態を更新")}>
          <RefreshCw size={12} /> {t("更新")}
        </Button>
      }>
        <Row className="gap-1.5">
          <Badge size="sm" className="font-mono">
            {status?.backend ?? (loading ? t("読み込み中…") : "—")}
          </Badge>
          <Badge size="sm">{status?.headless ? t("ヘッドレス") : t("ヘッドあり")}</Badge>
          <Badge size="sm">{status?.profiles ?? 0} {t("プロファイル")}</Badge>
          <Badge size="sm">{status?.artifacts ?? 0} {t("成果物")}</Badge>
        </Row>
        {status?.executable ? (
          <code className="break-all text-xs text-faint">{status.executable}</code>
        ) : loading ? (
          <span className="inline-flex items-center gap-1.5 text-xs text-muted">
            <Spinner size={12} /> {t("読み込み中…")}
          </span>
        ) : (
          <p className="m-0 text-xs text-muted">
            {t("Chromiumが見つかりません。システムのChromiumを入れるか、バンドル版を用意してください。")}
          </p>
        )}
      </Card>

      <Card title={t("ワークスペースのプロファイル")}>
        <Row className="gap-2">
          <Select
            value={knownProfile ? workspaceId : "new"}
            onChange={(e) => setWorkspaceId(e.target.value === "new" ? crypto.randomUUID() : e.target.value)}
          >
            {!knownProfile && <option value="new">{t("新規プロファイル")}</option>}
            {profiles.map((p) => (
              <option key={p.workspace_id} value={p.workspace_id}>
                {p.display_name} {p.provisioned ? "" : t("(未作成)")}
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
                setLog((l) => [{ id: nextId++, action: "info", ok: false, summary: errorMessage(e) }, ...l]);
              }
            }}
          >
            {t("リセット")}
          </Button>
        </Row>
      </Card>

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
        {needsText && (
          <Field>
            <Input
              aria-label={t("入力する文字")}
              placeholder={t("入力する文字")}
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
          </Field>
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

      <Card title={t("アクティビティ")}>
        {log.length === 0 ? (
          <EmptyState title={t("まだ操作がありません。")} />
        ) : (
          <ul className="flex flex-col">
            {log.map((e) => (
              <li key={e.id} className="border-t border-line py-2 first:border-0 first:pt-0">
                <Row className="gap-2">
                  <Badge size="sm" className="font-mono">{e.action}</Badge>
                  <span className={cn("text-xs", e.ok ? "text-success" : "text-danger")}>
                    {e.ok ? t("成功") : t("失敗")}: {e.summary}
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
                        size={12}
                        aria-hidden="true"
                        className="shrink-0 transition-transform group-open:rotate-90"
                      />
                      {t("出力")}
                    </summary>
                    <pre className={`${payloadBlock} mt-1.5 max-h-64 break-words`}>
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
        <Card title={t("既知のプロファイル")}>
          <ul className="flex flex-col gap-1 text-xs">
            {profiles.map((p) => (
              <li key={p.workspace_id} className="flex items-center gap-2">
                <span className="text-muted">{p.display_name}</span>
                <code className="font-mono text-xs text-faint">{p.workspace_id.slice(0, 8)}</code>
                <Badge tone={p.provisioned ? "success" : "neutral"} size="sm">
                  {p.provisioned ? t("準備完了") : t("空")}
                </Badge>
              </li>
            ))}
          </ul>
          <p className="m-0 text-xs text-muted">
            {t("各ワークスペースは独自のCookie・localStorage・キャッシュを持ち、エージェントのワークスペースと一致します。")}
          </p>
        </Card>
      )}
      {dialog}
    </Section>
  );
}
