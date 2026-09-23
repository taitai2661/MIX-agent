import { api } from "@/app/api";
import { Button } from "@/components/button";
import { useEffect, useState } from "react";

function keyBytes(value: string) {
  const raw = atob(value.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - value.length % 4) % 4));
  return Uint8Array.from(raw, char => char.charCodeAt(0));
}

export function PushControl() {
  const supported = typeof window !== "undefined" && window.isSecureContext && "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;
  const [enabled, setEnabled] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (!supported) return;
    let active = true;
    Promise.all([navigator.serviceWorker.getRegistration("/push-sw.js"), api<{ endpoints: string[] }>("/push/subscriptions")])
      .then(async ([registration, saved]) => {
        const subscription = await registration?.pushManager.getSubscription();
        if (active) setEnabled(!!subscription && saved.endpoints.includes(subscription.endpoint));
      }).catch(() => {});
    return () => { active = false; };
  }, [supported]);
  async function toggle() {
    setBusy(true); setError("");
    try {
      const registration = await navigator.serviceWorker.register("/push-sw.js", { scope: "/" });
      let subscription = await registration.pushManager.getSubscription();
      if (enabled) {
        if (subscription) {
          await api("/push/subscriptions", "DELETE", { endpoint: subscription.endpoint });
          await subscription.unsubscribe();
        }
        setEnabled(false);
      } else {
        if (Notification.permission === "denied") throw new Error("ブラウザ設定で通知を許可してください");
        const { public_key } = await api<{ public_key: string }>("/push/key");
        if (!subscription) subscription = await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(public_key) });
        await api("/push/subscriptions", "POST", subscription.toJSON());
        setEnabled(true);
      }
    } catch (cause) { setError(cause instanceof Error ? cause.message : "通知設定に失敗しました"); }
    finally { setBusy(false); }
  }
  return <section className="card"><h3>Webプッシュ通知</h3>
    <p>定期実行の完了・失敗などを、このブラウザに通知します。</p>
    {supported ? <Button type="button" variant="outline" disabled={busy} onClick={toggle}>{enabled ? "このブラウザの通知を停止" : "このブラウザで通知を有効化"}</Button>
      : <p>このブラウザでは利用できません。HTTPS または localhost で開いてください。</p>}
    {error && <p role="alert">{error}</p>}
  </section>;
}
