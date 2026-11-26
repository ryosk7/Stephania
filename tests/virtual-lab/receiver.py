"""Local TLS RTMP test peer. Only the client's fmt0/fmt3 128-byte chunks.
Records the received FLV tag bodies, deliberately disconnects first session.
Not a production ingest server."""
import socket
import ssl
import struct
import sys
import pathlib
import json
import time

out = pathlib.Path(sys.argv[1])
import os
sessions = int(os.environ.get("T3BOX_CI_SESSIONS", "2"))
first_limit = int(os.environ.get("T3BOX_CI_FIRST_LIMIT", "30"))
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(out / "server.pem", out / "server.key")

def read(s, n):
    data = bytearray()
    while len(data) < n:
        part = s.recv(n - len(data))
        if not part:
            raise EOFError
        data.extend(part)
    return bytes(data)

def msg(s):
    basic = read(s, 1)[0]
    assert basic >> 6 == 0
    h = read(s, 11)
    ts = int.from_bytes(h[:3], "big")
    length = int.from_bytes(h[3:6], "big")
    kind = h[6]
    if ts == 0xffffff:
        ts = int.from_bytes(read(s, 4), "big")
    assert 0 < length <= 4 * 1048576
    body = bytearray(read(s, min(128, length)))
    while len(body) < length:
        assert read(s, 1)[0] == (0xc0 | (basic & 63))
        if h[:3] == b"\xff\xff\xff":
            read(s, 4)
        body.extend(read(s, min(128, length - len(body))))
    return kind, ts, bytes(body)

def string(v):
    b = v.encode()
    return b"\x02" + struct.pack(">H",len(b)) + b

def number(v):
    return b"\x00" + struct.pack(">d",v)

def reply(s, stream, payload):
    header = b"\x03\0\0\0" + len(payload).to_bytes(3,"big") + b"\x14" + struct.pack("<I",stream)
    s.sendall(header + payload[:128])
    for i in range(128,len(payload),128):
        s.sendall(b"\xc3" + payload[i:i+128])

def serve(s, index):
    c = read(s,1537)
    assert c[0] == 3
    s.sendall(b"\x03" + bytes(1536) + c[1:])
    read(s,1536)
    for txn in (1,2):
        assert msg(s)[0] == 20
        reply(s,0,string("_result")+number(txn)+b"\x05"+(number(1) if txn == 2 else b"\x05"))
    assert msg(s)[0] == 20
    status = string("onStatus")+number(0)+b"\x05\x03\0\x04code"+string("NetStream.Publish.Start")+b"\0\0\x09"
    reply(s,1,status)
    counts = {"video":0,"audio":0}
    config = set()
    first_key = False
    last = {}
    first = {}
    max_skew_ms = 0
    video_arrivals = []
    video_ts = []
    video_bytes = 0
    with (out / f"session-{index}.flv").open("wb") as f:
        f.write(b"FLV\x01\x05\0\0\0\x09\0\0\0\0")
        while True:
            try:
                kind,ts,payload = msg(s)
            except EOFError:
                break
            assert kind in (8,9)
            assert ts >= last.get(kind,0)
            last[kind] = ts
            if payload[1] == 0:
                config.add(kind)
            else:
                assert kind in config
                key = "video" if kind == 9 else "audio"
                if kind == 9 and not counts[key]:
                    first_key = payload[0] >> 4 == 1
                counts[key] += 1
                if kind == 9:
                    video_arrivals.append(time.monotonic())
                    video_ts.append(ts)
                    video_bytes += len(payload)
                first.setdefault(key, ts)
                if 8 in last and 9 in last:
                    max_skew_ms = max(max_skew_ms, abs(last[8] - last[9]))
            f.write(bytes([kind])+len(payload).to_bytes(3,"big")+(ts&0xffffff).to_bytes(3,"big")+bytes([ts>>24])+b"\0\0\0"+payload+struct.pack(">I",11+len(payload)))
            if sessions > 1 and index == 1 and counts["video"] >= first_limit and counts["audio"] >= 20:
                break
    assert config == {8,9} and first_key and min(counts.values()) > 0
    counts["first_video_arrival_s"] = video_arrivals[0]
    counts["last_video_arrival_s"] = video_arrivals[-1]
    counts["video_media_span_ms"] = video_ts[-1] - video_ts[0]
    counts["video_max_gap_ms"] = max((b-a for a,b in zip(video_ts,video_ts[1:])),default=0)
    counts["video_payload_bytes"] = video_bytes
    counts["first_pts_ms"] = first
    counts["last_pts_ms"] = {"audio": last[8], "video": last[9]}
    counts["max_packet_timestamp_skew_ms"] = max_skew_ms
    (out / f"session-{index}.json").write_text(json.dumps(counts))

with socket.socket() as listener:
    listener.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
    listener.bind(("127.0.0.1",19443))
    listener.listen(2)
    listener.settimeout(90)
    (out / "ready").touch()
    for index in range(1,sessions+1):
        conn,_ = listener.accept()
        with conn:
            conn.settimeout(60)
            with ctx.wrap_socket(conn,server_side=True) as tls:
                serve(tls,index)
