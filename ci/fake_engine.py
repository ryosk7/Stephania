"""Minimal engine IPC responder for update tests.
Usage: fake_engine.py <socket> idle|streaming"""
import json, os, socket, struct, sys

path, mode = sys.argv[1], sys.argv[2]
if os.path.exists(path):
    os.unlink(path)
srv = socket.socket(socket.AF_UNIX)
srv.bind(path)
os.chmod(path, 0o666)
srv.listen(4)
while True:
    c, _ = srv.accept()
    try:
        n = struct.unpack(">I", c.recv(4))[0]
        req = json.loads(c.recv(n))
        body = json.dumps({"version": 1, "request_id": req["request_id"], "ok": True,
                           "result": {"state": "native_worker_active" if mode == "streaming" else "idle",
                                      "streaming_requested": mode == "streaming"}}).encode()
        c.sendall(struct.pack(">I", len(body)) + body)
    except Exception:
        pass
    finally:
        c.close()
