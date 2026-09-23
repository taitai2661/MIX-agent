import os

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from mix_agent.push import _derive, b64, encrypt, valid_endpoint


def test_push_payload_round_trip():
    recipient = ec.generate_private_key(ec.SECP256R1())
    public = recipient.public_key().public_bytes(
        serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint
    )
    auth = os.urandom(16)
    body = encrypt(b"hello", b64(public), b64(auth))
    salt, sender = body[:16], body[21:86]
    peer = ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), sender)
    shared = recipient.exchange(ec.ECDH(), peer)
    ikm = _derive(shared, auth, b"WebPush: info\x00" + public + sender, 32)
    plaintext = AESGCM(_derive(ikm, salt, b"Content-Encoding: aes128gcm\x00", 16)).decrypt(
        _derive(ikm, salt, b"Content-Encoding: nonce\x00", 12), body[86:], None
    )
    assert plaintext == b"hello\x02"


def test_push_endpoint_is_limited_to_browser_services():
    assert valid_endpoint("https://fcm.googleapis.com/fcm/send/id")
    assert not valid_endpoint("http://fcm.googleapis.com/fcm/send/id")
    assert not valid_endpoint("https://localhost/internal")
    assert not valid_endpoint("https://fcm.googleapis.com.evil.test/id")
    assert not valid_endpoint("https://fcm.googleapis.com:8443/id")
