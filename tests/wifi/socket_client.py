"""Drive t3box-provision over its socket (protocol v2) like the BLE adapter.

Used by tests/wifi/run-nm-e2e.sh against the real NetworkManager backend.
Usage: socket_client.py <socket> <setup-uri> <ssid> <good-psk>
"""
import hashlib
import hmac
import json
import socket
import struct
import sys

from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305

sock_path, uri, ssid, good_psk = sys.argv[1:5]


def mac(key, *parts):
    return hmac.new(key, b"".join(struct.pack(">I", len(p)) + p for p in parts), hashlib.sha256).digest()


def check(cond, message):
    print(("PASS " if cond else "FAIL ") + message, flush=True)
    if not cond:
        raise SystemExit(1)


secret = bytes.fromhex(uri.strip().split(":")[2])
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


nonce = bytes.fromhex(rpc({"op": "pairing.challenge"})["nonce"])
check(rpc({"op": "pairing.claim", "owner_id": "nm-ci", "proof": mac(secret, b"t3box-claim-v1", nonce, b"nm-ci").hex()}) == {"state": "CLAIMED"}, "claimed")
key = mac(secret, b"t3box-owner-key-v1", nonce, b"nm-ci")


def op_nonce():
    return bytes.fromhex(rpc({"op": "operation.challenge"})["nonce"])


def configure(cfg):
    n = op_nonce()
    sealed = ChaCha20Poly1305(mac(key, b"t3box-seal-v1")).encrypt(n[:12], json.dumps(cfg).encode(), b"wifi.configure")
    return rpc({"op": "wifi.configure", "sealed": sealed.hex(), "proof": mac(key, b"t3box-operation-v1", n, b"wifi.configure", sealed).hex()})


found = []
for attempt in range(5):  # hostapd may need a moment to start beaconing
    n = op_nonce()
    r = rpc({"op": "wifi.scan", "proof": mac(key, b"t3box-operation-v1", n, b"wifi.scan", b"").hex()})
    found = [x for x in r.get("networks", []) if x["ssid"] == ssid]
    if found:
        break
check(bool(found) and found[0]["security"] == "wpa-psk", f"real scan via NetworkManager found {found}")

r = configure({"ssid": ssid, "security": "wpa-psk", "psk": "wrong-passphrase"})
check(r == {"state": "FAILED"}, f"wrong PSK rejected by the AP {r}")
status = rpc({"op": "network.status"})
check(status.get("last_error") == "join_failed", f"status {status}")
r = configure({"ssid": ssid, "security": "wpa-psk", "psk": good_psk})
check(r == {"state": "CONNECTED"}, f"joined real AP {r}")
print("NM socket client: all checks passed", flush=True)
