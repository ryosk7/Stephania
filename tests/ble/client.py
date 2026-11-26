"""Independent BLE client for the T3Box provisioning protocol.

Runs over a real BlueZ stack (in CI: two btvirt virtual LE controllers) against
t3box-ble + t3box-provision. Implemented from contracts/ble-protocol.md only,
so it is a third implementation next to Rust and TypeScript.

Usage: client.py <setup-uri> [--adapter hci1] [--pair]
"""
import argparse
import asyncio
import hashlib
import hmac
import json
import struct
import sys

from bleak import BleakClient, BleakScanner

BASE = "-5d1e-4a6f-9b0e-3a1f2c4d5e60"
U = lambda n: f"7c3b{n:04x}{BASE}"
SERVICE = U(1)
DEVICE_INFO, PAIRING_CHALLENGE, PAIRING_RESPONSE = U(3), U(4), U(5)
SCAN_REQUEST, SCAN_RESULTS, WIFI_CONFIG, NETWORK_STATUS, OP_CHALLENGE = U(6), U(7), U(8), U(9), U(0xC)


def mac(key, *parts):
    data = b"".join(struct.pack(">I", len(p)) + p for p in parts)
    return hmac.new(key, data, hashlib.sha256).digest()


def parse_setup_uri(text):
    prefix, version, secret_hex, check = text.strip().lower().split(":")
    assert (prefix, version) == ("t3box-setup", "v1")
    secret = bytes.fromhex(secret_hex)
    assert hashlib.sha256(secret).hexdigest()[:8] == check
    return secret


class Session:
    def __init__(self, client, att):
        self.client = client
        self.att = att
        self.message_id = 0
        self.pending = {}

    async def subscribe(self, uuid):
        def on_notify(_char, data: bytearray):
            mid, index, count = data[0], data[1], data[2]
            print(f"notify {uuid[4:8]} {bytes(data).hex()}", flush=True)
            parts = self.pending.get((uuid, mid))
            if parts is None:
                print(f"unexpected notification for message {mid}", flush=True)
                return
            # The response's fragment count is independent of the request's.
            parts.setdefault("count", count)
            if parts["count"] != count or index in parts["parts"]:
                print(f"framing violation: have {sorted(parts['parts'])}/{parts['count']} got {index}/{count}", flush=True)
                return
            parts["parts"][index] = bytes(data[3:])
            if len(parts["parts"]) == count:
                parts["future"].set_result(b"".join(parts["parts"][i] for i in range(count)))
        await self.client.start_notify(uuid, on_notify)

    async def read(self, uuid):
        return json.loads(bytes(await self.client.read_gatt_char(uuid)))

    async def send(self, uuid, body, timeout=60):
        self.message_id = (self.message_id + 1) & 0xFF
        mid = self.message_id
        data = json.dumps(body, separators=(",", ":")).encode()
        chunk = self.att - 3
        frags = [data[i:i + chunk] for i in range(0, len(data), chunk)]
        assert len(frags) <= 64
        future = asyncio.get_running_loop().create_future()
        self.pending[(uuid, mid)] = {"parts": {}, "future": future}
        for i, f in enumerate(frags):
            await self.client.write_gatt_char(uuid, bytes([mid, i, len(frags)]) + f, response=True)
        result = json.loads(await asyncio.wait_for(future, timeout))
        del self.pending[(uuid, mid)]
        return result

    async def operation(self, key, uuid, operation, payload):
        nonce = bytes.fromhex((await self.read(OP_CHALLENGE))["nonce"])
        proof = mac(key, b"t3box-operation-v1", nonce, operation.encode(), payload)
        return await self.send(uuid, {"operation": operation, "payload_hex": payload.hex(), "proof": proof.hex()})


async def find(adapter, timeout=30):
    device = await BleakScanner.find_device_by_filter(
        lambda d, adv: SERVICE in [u.lower() for u in adv.service_uuids], timeout=timeout, adapter=adapter)
    if device is None:
        raise SystemExit("T3Box not found over BLE")
    return device


def check(cond, message):
    print(("PASS " if cond else "FAIL ") + message, flush=True)
    if not cond:
        raise SystemExit(1)


async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("setup_uri")
    ap.add_argument("--adapter", default="hci1")
    ap.add_argument("--pair", action="store_true")
    ap.add_argument("--min-mtu", action="store_true", help="fragment requests at the 20-byte minimum")
    args = ap.parse_args()
    secret = parse_setup_uri(args.setup_uri)
    device = await find(args.adapter)
    print(f"found {device.name} {device.address}", flush=True)
    async with BleakClient(device, adapter=args.adapter, timeout=30) as client:
        if args.pair:
            await client.pair()
        # bleak reports 23 unless the negotiated MTU is acquired explicitly.
        acquire = getattr(client._backend, "_acquire_mtu", None)
        if acquire is not None:
            await acquire()
        att = max(20, min(client.mtu_size, 512) - 3)
        if args.min_mtu:
            att = 20  # worst case: many small fragments in both directions
        s = Session(client, att)
        print(f"mtu {client.mtu_size} att {s.att}", flush=True)
        for uuid in (PAIRING_RESPONSE, SCAN_REQUEST, WIFI_CONFIG):
            await s.subscribe(uuid)

        info = await s.read(DEVICE_INFO)
        check(info["ownership"] == "UNCLAIMED" and info["protocol"] == 1, f"device info {info}")

        challenge = await s.read(PAIRING_CHALLENGE)
        check(challenge["state"] == "PAIRING", "pairing challenge issued (long read served from cache)")
        nonce = bytes.fromhex(challenge["nonce"])
        bad = await s.send(PAIRING_RESPONSE, {"owner_id": "ble-ci", "proof": mac(b"\0" * 16, b"t3box-claim-v1", nonce, b"ble-ci").hex()})
        check(bad == {"error": "bad_proof"}, f"wrong secret rejected via notified error {bad}")

        nonce = bytes.fromhex((await s.read(PAIRING_CHALLENGE))["nonce"])
        r = await s.send(PAIRING_RESPONSE, {"owner_id": "ble-ci", "proof": mac(secret, b"t3box-claim-v1", nonce, b"ble-ci").hex()})
        check(r == {"state": "CLAIMED"}, f"claimed {r}")
        key = mac(secret, b"t3box-owner-key-v1", nonce, b"ble-ci")
        check((await s.read(PAIRING_CHALLENGE))["state"] == "CLAIMED", "cannot be re-claimed")

        r = await s.operation(key, SCAN_REQUEST, "wifi.scan", b"{}")
        check(r.get("networks", 0) >= 1, f"scan {r}")
        networks = []
        for _ in range(r["pages"]):
            networks += (await s.read(SCAN_RESULTS))["networks"]
        ssid = networks[0]["ssid"]
        print(f"networks {networks}", flush=True)

        r = await s.operation(key, WIFI_CONFIG, "wifi.configure",
                              json.dumps({"ssid": "no-such-network", "security": "wpa-psk", "psk": "password1"}).encode())
        check(r == {"state": "FAILED"}, f"join failure reported {r}")
        r = await s.operation(key, WIFI_CONFIG, "wifi.configure",
                              json.dumps({"ssid": ssid, "security": "wpa-psk", "psk": "t3box-ci-pass"}).encode())
        check(r == {"state": "CONNECTED"}, f"joined {ssid} {r}")
        status = await s.read(NETWORK_STATUS)
        check(status["state"] == "CONNECTED", f"network status {status}")
        bogus = await s.send(WIFI_CONFIG, {"operation": "wifi.configure", "payload_hex": "7b7d", "proof": "00" * 32})
        check("error" in bogus, f"unauthenticated operation rejected {bogus}")
    print("BLE end-to-end: all checks passed", flush=True)


asyncio.run(main())
