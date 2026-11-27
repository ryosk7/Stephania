#!/usr/bin/env python3
"""Recompute the BLE crypto vectors (contracts/ble-protocol.md) independently
of the Rust and TypeScript implementations: HMAC-SHA256 with u32-BE length
prefixed fields, and ChaCha20-Poly1305 sealing. Inputs are read from the
existing files; only the derived values are rewritten.
  python3 contracts/fixtures/ble/generate.py [--check]"""
import hashlib, hmac, json, struct, sys
from pathlib import Path
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305

PREFIX = "stephania"
here = Path(__file__).parent

def mac(key: bytes, *fields: bytes) -> bytes:
    return hmac.new(key, b"".join(struct.pack(">I", len(f)) + f for f in fields), hashlib.sha256).digest()

def ctx(name: str) -> bytes:
    return f"{PREFIX}-{name}-v1".encode()

hv = json.loads((here / "hmac-vectors.json").read_text())
for v in hv["vectors"]:
    secret, nonce, owner = bytes.fromhex(v["setup_secret"]), bytes.fromhex(v["nonce"]), v["owner_id"].encode()
    v["claim_proof"] = mac(secret, ctx("claim"), nonce, owner).hex()
    key = mac(secret, ctx("owner-key"), nonce, owner)
    v["owner_key"] = key.hex()
    v["operation_proof"] = mac(key, ctx("operation"), bytes.fromhex(v["operation_nonce"]), v["operation"].encode(), bytes.fromhex(v["payload_hex"])).hex()

sv = json.loads((here / "seal-vectors.json").read_text())
sv["description"] = f"ChaCha20-Poly1305 seal vectors computed with Python cryptography: key = MAC(owner_key, '{PREFIX}-seal-v1'), nonce = operation_nonce[:12], AAD = operation."
for v in sv["vectors"]:
    key = mac(bytes.fromhex(v["owner_key"]), ctx("seal"))
    v["sealed_hex"] = ChaCha20Poly1305(key).encrypt(bytes.fromhex(v["operation_nonce"])[:12], bytes.fromhex(v["plaintext_hex"]), v["operation"].encode()).hex()

out = {"hmac-vectors.json": hv, "seal-vectors.json": sv}
stale = []
for name, data in out.items():
    text = json.dumps(data, indent=1) + "\n"
    if "--check" in sys.argv:
        if (here / name).read_text() != text: stale.append(name)
    else:
        (here / name).write_text(text)
if stale:
    sys.exit(f"out of date: {', '.join(stale)} (run contracts/fixtures/ble/generate.py)")
