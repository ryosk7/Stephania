//! Virtual-phone test against the real t3box-provision binary over its socket.
#![cfg(all(unix, feature = "ci-virtual"))]

use serde_json::{json, Value};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command};
use std::time::Duration;
use t3box::ipc::{read_frame, write_frame};
use t3box::provisioning::{claim_proof, derive_owner_key, fragment, hex, operation_proof, unhex, MIN_ATT_PAYLOAD};
use t3box::provisioning_gatt::Characteristic as C;

struct Guard(Child, std::path::PathBuf);
impl Drop for Guard {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); let _ = std::fs::remove_dir_all(&self.1); }
}

fn start(dir: &std::path::Path) -> Guard {
    let child = Command::new(env!("CARGO_BIN_EXE_t3box-provision"))
        .env("T3BOX_PROVISION_SOCKET", dir.join("provisioning.sock"))
        .env("T3BOX_CREDENTIALS_DIR", dir)
        .env("T3BOX_NETWORK_BACKEND", "virtual")
        .spawn().expect("spawn");
    for _ in 0..100 {
        if UnixStream::connect(dir.join("provisioning.sock")).is_ok() { break; }
        std::thread::sleep(Duration::from_millis(30));
    }
    Guard(child, dir.to_path_buf())
}

struct Phone { s: UnixStream, n: u32 }
impl Phone {
    fn call(&mut self, method: &str, params: Value) -> Value {
        self.n += 1;
        let body = serde_json::to_vec(&json!({"version":1,"request_id":format!("r{}",self.n),"method":method,"params":params})).unwrap();
        write_frame(&mut self.s, &body).unwrap();
        serde_json::from_slice(&read_frame(&mut self.s).unwrap().unwrap()).unwrap()
    }
    fn read(&mut self, c: C) -> Value {
        let r = self.call("gatt.read", json!({"uuid": c.uuid()}));
        assert_eq!(r["ok"], true, "{r}");
        let bytes: Vec<u8> = (0..r["result"]["value_hex"].as_str().unwrap().len()).step_by(2)
            .map(|i| u8::from_str_radix(&r["result"]["value_hex"].as_str().unwrap()[i..i + 2], 16).unwrap()).collect();
        serde_json::from_slice(&bytes).unwrap()
    }
    fn write(&mut self, c: C, message: &Value) -> Value {
        let frags = fragment(1, message.to_string().as_bytes(), MIN_ATT_PAYLOAD).unwrap();
        let mut last = Value::Null;
        for f in frags { last = self.call("gatt.write", json!({"uuid": c.uuid(), "fragment_hex": hex(&f)})); }
        last
    }
}

fn response(r: &Value) -> Value {
    let h = r["result"]["response_hex"].as_str().expect("response");
    serde_json::from_slice(&(0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect::<Vec<_>>()).unwrap()
}

#[test]
fn virtual_phone_claims_and_joins_over_socket() {
    let dir = std::env::temp_dir().join(format!("t3box-provision-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let guard = start(&dir);
    let secret = unhex::<16>(std::fs::read_to_string(dir.join("setup-secret")).unwrap().trim()).unwrap();
    let mut phone = Phone { s: UnixStream::connect(dir.join("provisioning.sock")).unwrap(), n: 0 };
    let challenge = phone.read(C::PairingChallenge);
    assert_eq!(challenge["state"], "PAIRING");
    let nonce = unhex::<32>(challenge["nonce"].as_str().unwrap()).unwrap();
    let r = phone.write(C::PairingResponse, &json!({"owner_id":"iphone-1","proof":hex(&claim_proof(&secret, &nonce, "iphone-1"))}));
    assert_eq!(response(&r)["state"], "CLAIMED");
    let key = derive_owner_key(&secret, &nonce, "iphone-1");

    let payload = br#"{"ssid":"virtual-lab","security":"wpa-psk","psk":"virtual-pass"}"#;
    let op_nonce = unhex::<32>(phone.read(C::OperationChallenge)["nonce"].as_str().unwrap()).unwrap();
    let r = phone.write(C::WifiConfiguration, &json!({"operation":"wifi.configure","payload_hex":hex(payload),"proof":hex(&operation_proof(&key, &op_nonce, "wifi.configure", payload))}));
    assert_eq!(response(&r)["state"], "CONNECTED");
    // Wrong proof is rejected with a stable code.
    let r = phone.write(C::WifiConfiguration, &json!({"operation":"wifi.configure","payload_hex":hex(payload),"proof":hex(&[0u8; 32])}));
    assert_eq!(r["error"]["code"], "no_pairing_in_progress");
    let unknown = phone.call("gatt.read", json!({"uuid": "00000000-0000-0000-0000-000000000000"}));
    assert_eq!(unknown["error"]["code"], "invalid_params");
    drop(phone);
    drop(guard);
}

#[test]
fn ownership_survives_process_restart() {
    let dir = std::env::temp_dir().join(format!("t3box-provision-restart-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let first = start(&dir);
    let secret = unhex::<16>(std::fs::read_to_string(dir.join("setup-secret")).unwrap().trim()).unwrap();
    let mut phone = Phone { s: UnixStream::connect(dir.join("provisioning.sock")).unwrap(), n: 0 };
    let nonce = unhex::<32>(phone.read(C::PairingChallenge)["nonce"].as_str().unwrap()).unwrap();
    phone.write(C::PairingResponse, &json!({"owner_id":"p","proof":hex(&claim_proof(&secret, &nonce, "p"))}));
    drop(phone);
    let mut child = first;
    let _ = child.0.kill();
    let _ = child.0.wait();
    std::mem::forget(child); // keep dir for the second process
    let _second = start(&dir);
    assert_eq!(std::fs::read_to_string(dir.join("setup-secret")).unwrap().trim(), hex(&secret));
    let mut phone = Phone { s: UnixStream::connect(dir.join("provisioning.sock")).unwrap(), n: 0 };
    assert_eq!(phone.read(C::PairingChallenge)["state"], "CLAIMED");
    assert_eq!(phone.read(C::DeviceInformation)["ownership"], "CLAIMED");
}
