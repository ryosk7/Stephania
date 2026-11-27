#!/usr/bin/env python3
"""Recompute the BLE protocol v3 vectors (contracts/ble-protocol.md)
independently of the Rust and TypeScript implementations, with Python
`cryptography` and `hmac`:

  claim-vectors.json  X25519 claim: shared secret, owner key, confirmation,
                      and an operation proof under the owner key
  seal-vectors.json   ChaCha20-Poly1305 sealing under the owner key

Inputs (private keys, nonces, payloads) are fixed below; only derived values
are computed.   python3 contracts/fixtures/ble/generate.py [--check]"""
import hashlib, hmac, json, struct, sys
from pathlib import Path
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

here = Path(__file__).parent

def mac(key: bytes, *fields: bytes) -> bytes:
    return hmac.new(key, b"".join(struct.pack(">I", len(f)) + f for f in fields), hashlib.sha256).digest()

def public(private: bytes) -> bytes:
    return X25519PrivateKey.from_private_bytes(private).public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)

CLAIMS = [
    {"device_private": bytes(range(32)), "phone_private": bytes(range(32, 64)), "nonce": bytes(range(64, 96)),
     "owner_id": "iphone-1", "operation": "wifi.configure", "operation_nonce": bytes(range(31, -1, -1)),
     "payload": b'{"ssid":"LT","security":"open"}'},
    {"device_private": bytes([0x5a] * 32), "phone_private": bytes([0xa5] * 32), "nonce": bytes([0x11] * 32),
     "owner_id": "pixel.8_a-B", "operation": "wifi.scan", "operation_nonce": bytes([0x22] * 32), "payload": b""},
]
claims = []
for c in CLAIMS:
    device_key, phone_key = public(c["device_private"]), public(c["phone_private"])
    shared = X25519PrivateKey.from_private_bytes(c["phone_private"]).exchange(
        X25519PrivateKey.from_private_bytes(c["device_private"]).public_key())
    owner_key = mac(shared, b"stephania-owner-key-v2", c["nonce"], c["owner_id"].encode(), device_key, phone_key)
    claims.append({
        "device_private": c["device_private"].hex(), "device_key": device_key.hex(),
        "phone_private": c["phone_private"].hex(), "phone_key": phone_key.hex(),
        "nonce": c["nonce"].hex(), "owner_id": c["owner_id"], "shared": shared.hex(), "owner_key": owner_key.hex(),
        "confirm": mac(owner_key, b"stephania-claim-confirm-v1", c["nonce"]).hex(),
        "operation": c["operation"], "operation_nonce": c["operation_nonce"].hex(), "payload_hex": c["payload"].hex(),
        "operation_proof": mac(owner_key, b"stephania-operation-v1", c["operation_nonce"], c["operation"].encode(), c["payload"]).hex(),
    })
cv = {"description": "Protocol v3 claim vectors computed with Python cryptography: shared = X25519(device, phone); "
      "owner_key = MAC(shared, 'stephania-owner-key-v2', nonce, owner_id, device_key, phone_key); "
      "confirm = MAC(owner_key, 'stephania-claim-confirm-v1', nonce); MAC fields are u32-BE length prefixed.",
      "vectors": claims}

sv = json.loads((here / "seal-vectors.json").read_text())
sv["description"] = "ChaCha20-Poly1305 seal vectors computed with Python cryptography: key = MAC(owner_key, 'stephania-seal-v1'), nonce = operation_nonce[:12], AAD = operation."
for v in sv["vectors"]:
    key = mac(bytes.fromhex(v["owner_key"]), b"stephania-seal-v1")
    v["sealed_hex"] = ChaCha20Poly1305(key).encrypt(bytes.fromhex(v["operation_nonce"])[:12], bytes.fromhex(v["plaintext_hex"]), v["operation"].encode()).hex()

out = {"claim-vectors.json": cv, "seal-vectors.json": sv}
stale = []
for name, data in out.items():
    text = json.dumps(data, indent=1) + "\n"
    if "--check" in sys.argv:
        if not (here / name).exists() or (here / name).read_text() != text: stale.append(name)
    else:
        (here / name).write_text(text)
if stale:
    sys.exit(f"out of date: {', '.join(stale)} (run contracts/fixtures/ble/generate.py)")
