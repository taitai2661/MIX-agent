self.addEventListener("push", (event) => {
  if (!event.data) return;
  let message;
  try { message = event.data.json(); } catch { return; }
  event.waitUntil(self.registration.showNotification(String(message.title || "MIX agent"), {
    body: "定期実行の通知があります",
    tag: String(message.id || "mix-notification"),
    data: { url: "/schedules" },
  }));
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil((async () => {
    const url = new URL(event.notification.data?.url || "/schedules", self.location.origin);
    const clients = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
    const existing = clients.find((client) => new URL(client.url).origin === url.origin);
    if (existing) { await existing.focus(); await existing.navigate(url.href); }
    else await self.clients.openWindow(url.href);
  })());
});
