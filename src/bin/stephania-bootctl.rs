//! RAUC custom bootloader backend for Raspberry Pi 5 tryboot, plus the
//! boot-time confirmation and install guard (crate::bootctl, docs/UPDATE.md).
//!
//! RAUC interface (system.conf `bootloader-custom-backend=`):
//!   get-primary | set-primary <slot> | get-state <slot> |
//!   set-state <slot> good|bad | get-current
//! Stephania commands:
//!   confirm   — after boot: health-check a trial, then `rauc status mark-*`
//!   guard     — RAUC pre-install handler: refuse while streaming
//!   health    — run the local health check only
//!   snapshot  — RAUC pre-install (after guard): online backup of the control DB
//!   restore-data — boot, before control: restore it after a rejected trial
//!   deadline  — trial still unconfirmed at the deadline: reject and reboot
//!
//! Environment: STEPHANIA_AUTOBOOT, STEPHANIA_BOOT_STATE, STEPHANIA_BOOT_SLOTS (A=2,B=3),
//! STEPHANIA_DEVICE_TREE, STEPHANIA_ENGINE_SOCKET, STEPHANIA_DATA_DIR, STEPHANIA_RAUC,
//! STEPHANIA_HEALTH_ATTEMPTS, STEPHANIA_REBOOT (command run after a failed trial),
//! STEPHANIA_CONTROL_DB, STEPHANIA_SNAPSHOT_DIR, STEPHANIA_SQLITE3.
#[cfg(unix)]
fn main() {
    use std::path::PathBuf;
    use stephania::bootctl::{self, BootCtl, Config, DataSnapshot, Decision};

    let env = |k: &str, d: &str| std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_owned());
    let die = |e: String| -> ! { eprintln!("stephania-bootctl: {e}"); std::process::exit(1) };
    let cfg = Config {
        autoboot: env("STEPHANIA_AUTOBOOT", "/boot/selector/autoboot.txt").into(),
        state: env("STEPHANIA_BOOT_STATE", "/data/stephania/update/bootctl.json").into(),
        slots: bootctl::parse_slots(&env("STEPHANIA_BOOT_SLOTS", "A=2,B=3")).unwrap_or_else(|e| die(e)),
        device_tree: env("STEPHANIA_DEVICE_TREE", "/proc/device-tree/chosen/bootloader").into(),
    };
    let socket = PathBuf::from(env("STEPHANIA_ENGINE_SOCKET", "/run/stephania/engine.sock"));
    let data = PathBuf::from(env("STEPHANIA_DATA_DIR", "/data/stephania"));
    let attempts: u32 = env("STEPHANIA_HEALTH_ATTEMPTS", "60").parse().unwrap_or(60);
    let ctl = BootCtl { cfg };
    let snapshot = DataSnapshot {
        database: env("STEPHANIA_CONTROL_DB", "/var/lib/stephania/control/control.db").into(),
        dir: env("STEPHANIA_SNAPSHOT_DIR", "/data/stephania/update/snapshot").into(),
        sqlite3: env("STEPHANIA_SQLITE3", "sqlite3"),
    };
    let reboot = || {
        let reboot = env("STEPHANIA_REBOOT", "systemctl reboot");
        let mut parts = reboot.split_whitespace();
        let _ = std::process::Command::new(parts.next().unwrap_or("true")).args(parts).status();
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_else(|| die("missing argument".into()));
    let rauc = |what: &str, slot: &str| {
        let rauc = env("STEPHANIA_RAUC", "rauc");
        let ok = std::process::Command::new(&rauc).args(["status", what, slot]).status().map(|s| s.success()).unwrap_or(false);
        if !ok { die(format!("{rauc} status {what} {slot} failed")); }
    };
    let result: Result<(), String> = match args.first().map(String::as_str) {
        Some("get-primary") => ctl.get_primary().map(|p| println!("{p}")),
        Some("set-primary") => ctl.set_primary(&arg(1)),
        Some("get-state") => ctl.get_state(&arg(1)).map(|s| println!("{s}")),
        Some("set-state") => match arg(2).as_str() {
            "good" => ctl.set_state(&arg(1), true),
            "bad" => ctl.set_state(&arg(1), false),
            other => Err(format!("state must be good or bad, not {other:?}")),
        },
        Some("get-current") => ctl.booted().and_then(|b| b.ok_or_else(|| "booted partition is not a known slot".into())).map(|b| println!("{b}")),
        Some("guard") => bootctl::install_guard(&socket),
        Some("health") => bootctl::health(&socket, &data, attempts, std::time::Duration::from_secs(1)),
        Some("confirm") => match ctl.decide() {
            Ok(Decision::Nothing) => { eprintln!("stephania-bootctl: no trial pending"); Ok(()) }
            Ok(Decision::TrialFailed(slot)) => {
                eprintln!("stephania-bootctl: trial of {slot} did not reach confirmation; marking it bad");
                rauc("mark-bad", "other");
                Ok(())
            }
            Ok(Decision::CheckTrial(slot)) => match bootctl::health(&socket, &data, attempts, std::time::Duration::from_secs(1)) {
                Ok(()) => {
                    eprintln!("stephania-bootctl: {slot} healthy; committing");
                    rauc("mark-good", "booted");
                    ctl.discard_snapshot_if_committed(&snapshot);
                    Ok(())
                }
                Err(e) => {
                    eprintln!("stephania-bootctl: {slot} unhealthy ({e}); rolling back");
                    rauc("mark-bad", "booted");
                    reboot();
                    Err("trial failed health checks".into())
                }
            },
            Err(e) => Err(e),
        },
        Some("snapshot") => snapshot.take().map(|taken| eprintln!("stephania-bootctl: data snapshot {}", if taken { "taken" } else { "skipped (no database)" })),
        Some("restore-data") => ctl.restore_data(&snapshot).map(|r| if r { eprintln!("stephania-bootctl: persistent data restored from the pre-update snapshot") }),
        Some("deadline") => match ctl.decide() {
            Ok(Decision::CheckTrial(slot)) => {
                eprintln!("stephania-bootctl: trial of {slot} not confirmed before the deadline; rolling back");
                rauc("mark-bad", "booted");
                reboot();
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        },
        _ => {
            eprintln!("usage: stephania-bootctl get-primary|set-primary <s>|get-state <s>|set-state <s> good|bad|get-current|confirm|guard|health|snapshot|restore-data|deadline");
            std::process::exit(2);
        }
    };
    if let Err(e) = result { die(e); }
}

#[cfg(not(unix))]
fn main() { std::process::exit(2); }
