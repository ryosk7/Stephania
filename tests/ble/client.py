"""Independent BLE client for T3Box provisioning protocol v2.

Runs over a real BlueZ stack (in CI: two btvirt virtual LE controllers)
against t3box-ble + t3box-provision. Written from contracts/ble-protocol.md
only, so it is a third implementation next to Rust and TypeScript.

Phases:
  claim   wrong secret, claim, lost-response replay, scan, join fail/ok,
          unauthenticated rejection (fresh device)
  resume  after bluetoothd restart: the device is found again and still owned

Usage: client.py <setup-uri> <phase> [--adapter hci1] [--min-mtu]
"""
import argparse
import asyncio
import hashlib
import hmac
import json
import struct
import sys

from bleak import BleakClient, BleakScanner
from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305

SERVICE = "7c3b0100-5d1e-4a6f-9b0e-3a1f2c4d5e60"
INFO = "7c3b0101-5d1e-4a6f-9b0e-3a1f2c4d5e60"
RPC = "7c3b0102-5d1e-4a6f-9b0e-3a1f2c4d5e60"


def mac(key, *parts):
    data = b"".join(struct.pack(">I", len(p)) + p for p in parts)
    return hmac.new(key, data, hashlib.sha256).digest()


def seal(owner_key, op_nonce, operation, plaintext):
    key = mac(owner_key, b"t3box-seal-v1")
    return ChaCha20Poly1305(key).encrypt(op_nonce[:12], plaintext, operation.encode())


def parse_setup_uri(text):
    prefix, version, secret_hex, check = text.strip().lower().split(":")
    assert (prefix, version) == ("t3box-setup", "v1")
    secret = bytes.fromhex(secret_hex)
    assert hashlib.sha256(secret).hexdigest()[:8] == check
    return secret


def check(cond, message):
    print(("PASS " if cond else "FAIL ") + message, flush=True)
    if not cond:
        raise SystemExit(1)


class Rpc:
    def __init__(self, client, att):
        self.client = client
        self.att = att
        self.next_id = 0
        self.pending = {}

    async def start(self):
        def on_indication(_c, data: bytearray):
            mid, index, count = data[0], data[1], data[2]
            entry = self.pending.get(mid)
            if entry is None:
                return  # stale or another request's response: ignore
            entry.setdefault("count", count)
            if entry["count"] != count:
                entry["future"].set_exception(RuntimeError("inconsistent fragment count"))
                return
            # Identical duplicate fragments are tolerated (idempotent).
            entry["parts"][index] = bytes(data[3:])
            if len(entry["parts"]) == count and not entry["future"].done():
                entry["future"].set_result(b"".join(entry["parts"][i] for i in range(count)))
        await self.client.start_notify(RPC, on_indication)

    async def send(self, body, mid=None, timeout=60):
        if mid is None:
            self.next_id = (self.next_id + 1) & 0xFF
            mid = self.next_id
        data = json.dumps(body, separators=(",", ":")).encode()
        chunk = self.att - 3
        frags = [data[i:i + chunk] for i in range(0, len(data), chunk)]
        assert len(frags) <= 255
        future = asyncio.get_running_loop().create_future()
        self.pending[mid] = {"parts": {}, "future": future}
        try:
            for i, f in enumerate(frags):
                await self.client.write_gatt_char(RPC, bytes([mid, i, len(frags)]) + f, response=True)
            return json.loads(await asyncio.wait_for(future, timeout))
        finally:
            self.pending.pop(mid, None)

    async def nonce(self, op):
        return bytes.fromhex((await self.send({"op": op}))["nonce"])


async def connect(adapter, min_mtu):
    device = await BleakScanner.find_device_by_filter(
        lambda d, adv: SERVICE in [u.lower() for u in adv.service_uuids], timeout=30, adapter=adapter)
    if device is None:
        raise SystemExit("T3Box not found over BLE")
    print(f"found {device.name} {device.address}", flush=True)
    client = BleakClient(device, adapter=adapter, timeout=30)
    await client.connect()
    acquire = getattr(client._backend, "_acquire_mtu", None)
    if acquire is not None:
        await acquire()
    att = 20 if min_mtu else max(20, min(client.mtu_size, 512) - 3)
    print(f"mtu {client.mtu_size} att {att}", flush=True)
    info = json.loads(bytes(await client.read_gatt_char(INFO)))
    check(info == {"v": 2, "p": "t3box"}, f"static info read {info}")
    rpc = Rpc(client, att)
    await rpc.start()
    return client, rpc


async def phase_claim(rpc, secret):
    info = await rpc.send({"op": "device.info"})
    check(info["ownership"] == "UNCLAIMED" and info["protocol"] == 2, f"device info {info}")

    nonce = await rpc.nonce("pairing.challenge")
    bad = await rpc.send({"op": "pairing.claim", "owner_id": "ble-ci",
                          "proof": mac(b"\0" * 16, b"t3box-claim-v1", nonce, b"ble-ci").hex()})
    check(bad == {"error": "bad_proof"}, f"wrong secret rejected {bad}")

    nonce = await rpc.nonce("pairing.challenge")
    claim = {"op": "pairing.claim", "owner_id": "ble-ci", "proof": mac(secret, b"t3box-claim-v1", nonce, b"ble-ci").hex()}
    r = await rpc.send(claim, mid=200)
    check(r == {"state": "CLAIMED"}, f"claimed {r}")
    # Simulated lost indication: resend the identical request with the same id.
    r = await rpc.send(claim, mid=200)
    check(r == {"state": "CLAIMED"}, f"identical retry replayed, not rejected {r}")
    r = await rpc.send(claim)
    check(r == {"error": "not_claimable"}, f"new claim refused {r}")
    key = mac(secret, b"t3box-owner-key-v1", nonce, b"ble-ci")

    n = await rpc.nonce("operation.challenge")
    r = await rpc.send({"op": "wifi.scan", "proof": mac(key, b"t3box-operation-v1", n, b"wifi.scan", b"").hex()})
    check(len(r.get("networks", [])) >= 1, f"scan {r}")
    ssid = r["networks"][0]["ssid"]

    async def configure(cfg):
        n = await rpc.nonce("operation.challenge")
        sealed = seal(key, n, "wifi.configure", json.dumps(cfg).encode())
        proof = mac(key, b"t3box-operation-v1", n, b"wifi.configure", sealed)
        return await rpc.send({"op": "wifi.configure", "sealed": sealed.hex(), "proof": proof.hex()})

    r = await configure({"ssid": "no-such-network", "security": "wpa-psk", "psk": "password1"})
    check(r == {"state": "FAILED"}, f"join failure reported {r}")
    r = await configure({"ssid": ssid, "security": "wpa-psk", "psk": "t3box-ci-pass"})
    check(r == {"state": "CONNECTED"}, f"joined {ssid} {r}")
    check(await rpc.send({"op": "network.status"}) == {"state": "CONNECTED"}, "network status")
    bogus = await rpc.send({"op": "wifi.configure", "sealed": "00" * 20, "proof": "00" * 32})
    check("error" in bogus, f"unauthenticated operation rejected {bogus}")


async def phase_resume(rpc):
    info = await rpc.send({"op": "device.info"})
    check(info["ownership"] == "CLAIMED" and info["network"] == "CONNECTED",
          f"after bluetoothd restart: same device state {info}")
    check(await rpc.send({"op": "pairing.challenge"}) == {"state": "CLAIMED"}, "still not re-claimable")


async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("setup_uri")
    ap.add_argument("phase", choices=["claim", "resume"])
    ap.add_argument("--adapter", default="hci1")
    ap.add_argument("--min-mtu", action="store_true")
    args = ap.parse_args()
    secret = parse_setup_uri(args.setup_uri)
    client, rpc = await connect(args.adapter, args.min_mtu)
    try:
        if args.phase == "claim":
            await phase_claim(rpc, secret)
        else:
            await phase_resume(rpc)
    finally:
        await client.disconnect()
    print(f"BLE {args.phase}: all checks passed", flush=True)


asyncio.run(main())
