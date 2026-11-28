"""Drive stephania-provision over its socket (protocol v3) like the BLE adapter.

Used by tests/wifi/run-nm-e2e.sh against the real NetworkManager backend.
Usage: socket_client.py <socket> <ssid> <good-psk> <wpa3-ssid> <wpa3-psk>
The environment also has an open access point named <ssid> (an evil twin).
"""
import hashlib
import hmac
import json
import socket
import struct
import sys

from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat

sock_path, ssid, good_psk, sae_ssid, sae_psk = sys.argv[1:6]


def mac(key, *parts):
    return hmac.new(key, b"".join(struct.pack(">I", len(p)) + p for p in parts), hashlib.sha256).digest()


def check(cond, message):
    print(("PASS " if cond else "FAIL ") + message, flush=True)
    if not cond:
        raise SystemExit(1)


s = socket.socket(socket.AF_UNIX)
s.connect(sock_path)
s.settimeout(90)
state = {"n": 0, "id": 0}


def frame_call(method, params):
    state["n"] += 1
    body = json.dumps({"version": 1, "request_id": f"w{state['n']}", "method": method, "params": params}).encode()
    s.sendall(struct.pack(">I", len(body)) + body)
    hdr = b""
    while len(hdr) < 4:
        hdr += s.recv(4 - len(hdr))
    n = struct.unpack(">I", hdr)[0]
    data = b""
    while len(data) < n:
        data += s.recv(n - len(data))
    return json.loads(data)


def rpc(body):
    state["id"] = (state["id"] % 255) + 1
    mid = state["id"]
    data = json.dumps(body).encode()
    frags = [data[i:i + 17] for i in range(0, len(data), 17)]
    last = None
    for i, f in enumerate(frags):
        last = frame_call("gatt.request", {"peer": "nm-e2e", "fragment_hex": (bytes([mid, i, len(frags)]) + f).hex()})
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


key = claim(b"nm-ci")


def op_nonce():
    return bytes.fromhex(rpc({"op": "operation.challenge"})["nonce"])


def configure(cfg):
    n = op_nonce()
    sealed = ChaCha20Poly1305(mac(key, b"stephania-seal-v1")).encrypt(n[:12], json.dumps(cfg).encode(), b"wifi.configure")
    return rpc({"op": "wifi.configure", "sealed": sealed.hex(), "proof": mac(key, b"stephania-operation-v1", n, b"wifi.configure", sealed).hex()})


found, networks = [], []
for attempt in range(8):  # hostapd may need a moment to start beaconing
    n = op_nonce()
    r = rpc({"op": "wifi.scan", "proof": mac(key, b"stephania-operation-v1", n, b"wifi.scan", b"").hex()})
    networks = r.get("networks", [])
    found = sorted(x["security"] for x in networks if x["ssid"] == ssid)
    if found == ["open", "wpa-psk"] and any(x["ssid"] == sae_ssid for x in networks):
        break
check(found == ["open", "wpa-psk"], f"real scan lists the secured network and its open twin separately: {networks}")
check([x["security"] for x in networks if x["ssid"] == sae_ssid] == ["wpa-psk"], f"WPA3-only network listed as a passphrase network: {networks}")

r = configure({"ssid": ssid, "security": "open"})
check(r == {"error": "weaker_security"}, f"open twin of a secured network refused {r}")
r = configure({"ssid": sae_ssid, "security": "wpa-psk", "psk": sae_psk})
check(r == {"state": "CONNECTED"}, f"joined WPA3-only (SAE) AP {r}")

r = configure({"ssid": ssid, "security": "wpa-psk", "psk": "wrong-passphrase"})
check(r == {"state": "FAILED"}, f"wrong PSK rejected by the AP {r}")
status = rpc({"op": "network.status"})
check(status.get("last_error") == "join_failed", f"status {status}")
r = configure({"ssid": ssid, "security": "wpa-psk", "psk": good_psk})
check(r == {"state": "CONNECTED"}, f"joined real AP {r}")
print("NM socket client: all checks passed", flush=True)
