"""VAPID authenticated, aes128gcm Web Push delivery for saved notifications."""
import asyncio
import base64
import json
import logging
import os
import time
from urllib.parse import urlsplit

import httpx
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from sqlalchemy import or_, select

from mix_agent import config
from mix_agent.db.models import Notification, PushSubscription
from mix_agent.db.session import SessionLocal

log = logging.getLogger(__name__)
PUSH_HOSTS = (
    "fcm.googleapis.com", "push.services.mozilla.com",
    "updates.push.services.mozilla.com", "web.push.apple.com",
)


def b64(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def unb64(value):
    return base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))


def valid_endpoint(value):
    try:
        url = urlsplit(value)
        return (url.scheme == "https" and url.hostname in PUSH_HOSTS and
                url.port in (None, 443) and not url.username and not url.password and
                not url.fragment and len(value) <= 2048)
    except ValueError:
        return False


def private_key():
    path = config.KEYS / "webpush-vapid.pem"
    try:
        return serialization.load_pem_private_key(path.read_bytes(), password=None)
    except FileNotFoundError:
        key = ec.generate_private_key(ec.SECP256R1())
        payload = key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                                    serialization.NoEncryption())
        try:
            with path.open("xb") as file:
                os.chmod(path, 0o600)
                file.write(payload)
            return key
        except FileExistsError:
            return serialization.load_pem_private_key(path.read_bytes(), password=None)


def public_key():
    return b64(private_key().public_key().public_bytes(serialization.Encoding.X962,
                                                       serialization.PublicFormat.UncompressedPoint))


def _derive(secret, salt, info, length):
    return HKDF(algorithm=hashes.SHA256(), length=length, salt=salt, info=info).derive(secret)


def encrypt(payload, client_key, auth):
    recipient = unb64(client_key)
    if len(recipient) != 65 or len(unb64(auth)) != 16:
        raise ValueError("Invalid subscription keys")
    peer = ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), recipient)
    local = ec.generate_private_key(ec.SECP256R1())
    sender = local.public_key().public_bytes(serialization.Encoding.X962,
                                              serialization.PublicFormat.UncompressedPoint)
    shared = local.exchange(ec.ECDH(), peer)
    ikm = _derive(shared, unb64(auth), b"WebPush: info\x00" + recipient + sender, 32)
    salt = os.urandom(16)
    cek = _derive(ikm, salt, b"Content-Encoding: aes128gcm\x00", 16)
    nonce = _derive(ikm, salt, b"Content-Encoding: nonce\x00", 12)
    encrypted = AESGCM(cek).encrypt(nonce, payload + b"\x02", None)
    return salt + (4096).to_bytes(4, "big") + bytes((len(sender),)) + sender + encrypted


def headers(endpoint):
    key = private_key()
    point = public_key()
    origin = f"https://{urlsplit(endpoint).hostname}"
    head = b64(b'{"typ":"JWT","alg":"ES256"}')
    subject = os.getenv("WEB_PUSH_SUBJECT", config.PUBLIC_ORIGIN if config.PUBLIC_ORIGIN.startswith("https://")
                        else "mailto:admin@example.com")
    claims = b64(json.dumps({"aud": origin, "exp": int(time.time()) + 3600,
                             "sub": subject}, separators=(",", ":")).encode())
    signed = (head + "." + claims).encode()
    signature = key.sign(signed, ec.ECDSA(hashes.SHA256()))
    from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature
    r, s = decode_dss_signature(signature)
    token = head + "." + claims + "." + b64(r.to_bytes(32, "big") + s.to_bytes(32, "big"))
    return {"Authorization": f"vapid t={token}, k={point}", "TTL": "86400",
            "Content-Encoding": "aes128gcm", "Content-Type": "application/octet-stream"}


def deliver_pending():
    with SessionLocal() as db, httpx.Client(timeout=10, follow_redirects=False) as client:
        for sub in db.scalars(select(PushSubscription)).all():
            pending = db.scalars(select(Notification).where(
                Notification.owner_id == sub.owner_id,
                or_(Notification.created_at > sub.cursor_at,
                    (Notification.created_at == sub.cursor_at) & (Notification.id > sub.cursor_id)),
            ).order_by(Notification.created_at, Notification.id).limit(20)).all()
            for notification in pending:
                try:
                    data = json.dumps({"id": notification.id,
                                       "title": notification.data.get("title", "MIX agent"),
                                       "kind": notification.data.get("kind", ""),
                                       "url": "/schedules"}, ensure_ascii=False).encode()
                    response = client.post(sub.endpoint, content=encrypt(data, sub.data["p256dh"], sub.data["auth"]),
                                           headers=headers(sub.endpoint))
                    if response.status_code in (404, 410):
                        db.delete(sub)
                        db.commit()
                        break
                    response.raise_for_status()
                except (httpx.HTTPError, ValueError, KeyError) as exc:
                    log.warning("Web Push delivery failed: %s", type(exc).__name__)
                    break
                sub.cursor_at, sub.cursor_id = notification.created_at, notification.id
                db.commit()


async def worker():
    while True:
        try:
            await asyncio.to_thread(deliver_pending)
        except Exception:
            log.exception("Web Push worker failed")
        await asyncio.sleep(15)
