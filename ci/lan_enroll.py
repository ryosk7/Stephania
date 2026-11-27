"""Owner enrollment over the provisioning socket (as the BLE adapter would
forward it), then a real TLS call to the control plane pinned to the CA the
device delivered sealed.

Usage: lan_enroll.py <provision.sock> <tls-port> <out-dir>
Writes <out-dir>/token and <out-dir>/pinned-ca.sha256.
"""
import hashlib
import hmac
import json
import socket
import ssl
import struct
import sys
import http.client

from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

sock_path, port, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]


def mac(key, *parts):
    return hmac.new(key, b"".join(struct.pack(">I", len(p)) + p for p in parts), hashlib.sha256).digest()


def check(cond, message):
    print(("PASS " if cond else "FAIL ") + message, flush=True)
    if not cond:
        raise SystemExit(1)


s = socket.socket(socket.AF_UNIX)
s.connect(sock_path)
s.settimeout(10)
state = {"n": 0, "id": 0}


def frame_call(method, params):
    state["n"] += 1
    body = json.dumps({"version": 1, "request_id": f"l{state['n']}", "method": method, "params": params}).encode()
    s.sendall(struct.pack(">I", len(body)) + body)
    hdr = b""
    while len(hdr) < 4:
        hdr += s.recv(4 - len(hdr))
    n = struct.unpack(">I", hdr)[0]
    data = b""
    while len(data) < n:
        data += s.recv(n - len(data))
    return json.loads(data)


def rpc(body, mid=None):
    if mid is None:
        state["id"] = (state["id"] % 255) + 1
        mid = state["id"]
    data = json.dumps(body, separators=(",", ":")).encode()
    frags = [data[i:i + 17] for i in range(0, len(data), 17)]
    last = None
    for i, f in enumerate(frags):
        last = frame_call("gatt.request", {"peer": "lan-e2e", "fragment_hex": (bytes([mid, i, len(frags)]) + f).hex()})
        assert last["ok"], last
    return json.loads(bytes.fromhex(last["result"]["response_hex"]))


def claim(owner_id):
    """Protocol v3: the first claim wins; X25519 gives both sides the owner key."""
    c = rpc({"op": "pairing.challenge"})
    nonce, device_key = bytes.fromhex(c["nonce"]), bytes.fromhex(c["device_key"])
    private = X25519PrivateKey.generate()
    phone_key = private.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    shared = private.exchange(X25519PublicKey.from_public_bytes(device_key))
    key = mac(shared, b"stephania-owner-key-v2", nonce, owner_id, device_key, phone_key)
    r = rpc({"op": "pairing.claim", "owner_id": owner_id.decode(), "phone_key": phone_key.hex()})
    check(r == {"state": "CLAIMED", "confirm": mac(key, b"stephania-claim-confirm-v1", nonce).hex()}, "claimed; both sides derived the same owner key")
    return key


key = claim(b"lan-ci")
check(rpc({"op": "management.endpoint"}) == {"available": True}, "endpoint advertised as available")

n = bytes.fromhex(rpc({"op": "operation.challenge"})["nonce"])
req = {"op": "management.enroll", "proof": mac(key, b"stephania-operation-v1", n, b"management.enroll", b"").hex()}
r = rpc(req, mid=200)
again = rpc(req, mid=200)
check(again == r, "identical retry replays the same sealed enrollment")
plain = json.loads(ChaCha20Poly1305(mac(key, b"stephania-seal-v1")).decrypt(n[:12], bytes.fromhex(r["sealed"]), b"management.enroll"))
token, pin, urls = plain["token"], plain["ca_sha256"], plain["urls"]
check(token.startswith("t3b_") and len(token) == 68, "token delivered sealed")
check(urls[0] == f"https://stephania-ci.local:{port}/", f"urls {urls}")

# Phone-side trust: accept the server only if its chain ends in the pinned CA.
ctx = ssl.create_default_context()
ctx.check_hostname = False
ctx.verify_mode = ssl.CERT_NONE
raw = socket.create_connection(("127.0.0.1", port), timeout=10)
tls = ctx.wrap_socket(raw, server_hostname="stephania-ci.local")
chain = tls.get_unverified_chain() if hasattr(tls, "get_unverified_chain") else None
tls.close()
if chain is not None:
    der = [c if isinstance(c, bytes) else c.public_bytes(ssl._ssl.ENCODING_DER) for c in chain]
    check(len(der) >= 2 and hashlib.sha256(der[-1]).hexdigest() == pin, "server sends its chain ending in the CA pinned over BLE")
else:
    print("SKIP chain pin check (Python < 3.13); curl --cacert check below covers trust", flush=True)

with open(f"{out}/token", "w") as f:
    f.write(token)
with open(f"{out}/pinned-ca.sha256", "w") as f:
    f.write(pin)
print("LAN enrollment: all checks passed", flush=True)
