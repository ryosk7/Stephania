//! Protocol v3 against the real stephania-provision binary over its socket.
#![cfg(all(unix, feature = "ci-virtual"))]

use serde_json::{json, Value};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command};
use std::time::Duration;
use stephania::ipc::{read_frame, write_frame};
use stephania::provisioning::{claim_confirmation, client_claim, fragment, hex, operation_proof, seal, unhex, PairingChallenge, MIN_ATT_PAYLOAD};

struct Guard(Child, std::path::PathBuf, bool);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        if self.2 { let _ = std::fs::remove_dir_all(&self.1); }
    }
}

fn start(dir: &std::path::Path, cleanup: bool) -> Guard {
    let child = Command::new(env!("CARGO_BIN_EXE_stephania-provision"))
        .env("STEPHANIA_PROVISION_SOCKET", dir.join("provisioning.sock"))
        .env("STEPHANIA_CREDENTIALS_DIR", dir)
        .env("STEPHANIA_NETWORK_BACKEND", "virtual")
        .spawn().expect("spawn");
    for _ in 0..100 {
        if UnixStream::connect(dir.join("provisioning.sock")).is_ok() { break; }
        std::thread::sleep(Duration::from_millis(30));
    }
    Guard(child, dir.to_path_buf(), cleanup)
}

fn tempdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("stephania-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Plays the BLE adapter's role: forwards fragments, reassembles nothing
/// itself (the device core returns the whole response).
struct Phone { s: UnixStream, n: u32, id: u8, peer: String }
impl Phone {
    fn connect(dir: &std::path::Path, peer: &str) -> Self {
        Phone { s: UnixStream::connect(dir.join("provisioning.sock")).unwrap(), n: 0, id: 0, peer: peer.into() }
    }
    fn call(&mut self, method: &str, params: Value) -> Value {
        self.n += 1;
        let body = serde_json::to_vec(&json!({"version":1,"request_id":format!("r{}",self.n),"method":method,"params":params})).unwrap();
        write_frame(&mut self.s, &body).unwrap();
        serde_json::from_slice(&read_frame(&mut self.s).unwrap().unwrap()).unwrap()
    }
    fn send_with_id(&mut self, id: u8, body: &Value) -> Value {
        let mut last = Value::Null;
        for f in fragment(id, body.to_string().as_bytes(), MIN_ATT_PAYLOAD).unwrap() {
            last = self.call("gatt.request", json!({"peer": self.peer, "fragment_hex": hex(&f)}));
            assert_eq!(last["ok"], true, "{last}");
        }
        assert_eq!(last["result"]["message_id"], id);
        let h = last["result"]["response_hex"].as_str().expect("response");
        serde_json::from_slice(&(0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect::<Vec<_>>()).unwrap()
    }
    fn send(&mut self, body: Value) -> Value { self.id = self.id.wrapping_add(1); let id = self.id; self.send_with_id(id, &body) }
    fn nonce(&mut self, op: &str) -> [u8; 32] {
        unhex::<32>(self.send(json!({"op": op}))["nonce"].as_str().unwrap()).unwrap()
    }
    fn challenge(&mut self) -> PairingChallenge {
        let c = self.send(json!({"op": "pairing.challenge"}));
        PairingChallenge { nonce: unhex(c["nonce"].as_str().unwrap()).unwrap(), device_key: unhex(c["device_key"].as_str().unwrap()).unwrap() }
    }
    /// First-come claim: X25519 with the device's per-challenge key.
    fn claim(&mut self, owner_id: &str) -> [u8; 32] {
        let c = self.challenge();
        let (phone_key, key) = client_claim(&c, owner_id).unwrap();
        let r = self.send(json!({"op":"pairing.claim","owner_id":owner_id,"phone_key":hex(&phone_key)}));
        assert_eq!(r, json!({"state":"CLAIMED","confirm":hex(&claim_confirmation(&key, &c.nonce))}));
        key
    }
}

#[test]
fn claim_scan_join_over_socket() {
    let dir = tempdir("prov-v3");
    let _guard = start(&dir, true);
    let mut p = Phone::connect(&dir, "AA:BB:CC:DD:EE:01");
    assert_eq!(p.send(json!({"op":"device.info"}))["protocol"], 3);
    let first = p.challenge();
    assert_eq!(p.send(json!({"op":"pairing.claim","owner_id":"x","phone_key":hex(&[0u8;32])})), json!({"error":"bad_key"}));
    assert_ne!(first.device_key, p.challenge().device_key, "fresh device key per challenge");
    let key = p.claim("first-phone");
    assert_eq!(p.send(json!({"op":"pairing.challenge"})), json!({"state":"CLAIMED"}));

    let n = p.nonce("operation.challenge");
    let r = p.send(json!({"op":"wifi.scan","proof":hex(&operation_proof(&key, &n, "wifi.scan", b""))}));
    assert_eq!(r["networks"][0]["ssid"], "virtual-lab");

    let n = p.nonce("operation.challenge");
    let sealed = seal(&key, &n, "wifi.configure", br#"{"ssid":"virtual-lab","security":"wpa-psk","psk":"virtual-pass"}"#);
    let req = json!({"op":"wifi.configure","sealed":hex(&sealed),"proof":hex(&operation_proof(&key, &n, "wifi.configure", &sealed))});
    assert_eq!(p.send_with_id(200, &req), json!({"state":"CONNECTED"}));
    // Lost indication: the phone reconnects over BLE with a new address (the
    // adapter keeps its single socket) and resends the identical request; it
    // is replayed, not rejected as a reused nonce.
    assert_eq!(p.call("gatt.disconnect", json!({"peer":"AA:BB:CC:DD:EE:01"}))["ok"], true);
    p.peer = "AA:BB:CC:DD:EE:02".into();
    assert_eq!(p.send_with_id(200, &req), json!({"state":"CONNECTED"}));
    assert!(p.call("gatt.request", json!({"peer":"bad peer!","fragment_hex":"00"}))["error"]["code"] == "invalid_params");
}

#[test]
fn ownership_survives_process_restart() {
    let dir = tempdir("prov-restart");
    let first = start(&dir, false);
    let mut p = Phone::connect(&dir, "peer-1");
    p.claim("p");
    drop(p);
    drop(first);
    let _second = start(&dir, true);
    let mut p = Phone::connect(&dir, "peer-1");
    assert_eq!(p.send(json!({"op":"pairing.challenge"})), json!({"state":"CLAIMED"}));
}
