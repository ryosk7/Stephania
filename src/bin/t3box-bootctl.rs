//! RAUC custom bootloader backend for Raspberry Pi 5 tryboot, plus the
//! boot-time confirmation and install guard (crate::bootctl, docs/UPDATE.md).
//!
//! RAUC interface (system.conf `bootloader-custom-backend=`):
//!   get-primary | set-primary <slot> | get-state <slot> |
//!   set-state <slot> good|bad | get-current
//! T3Box commands:
//!   confirm   — after boot: health-check a trial, then `rauc status mark-*`
//!   guard     — RAUC pre-install handler: refuse while streaming
//!   health    — run the local health check only
//!
//! Environment: T3BOX_AUTOBOOT, T3BOX_BOOT_STATE, T3BOX_BOOT_SLOTS (A=2,B=3),
//! T3BOX_DEVICE_TREE, T3BOX_ENGINE_SOCKET, T3BOX_DATA_DIR, T3BOX_RAUC,
//! T3BOX_HEALTH_ATTEMPTS, T3BOX_REBOOT (command run after a failed trial).
#[cfg(unix)]
fn main() {
    use std::path::PathBuf;
    use t3box::bootctl::{self, BootCtl, Config, Decision};

    let env = |k: &str, d: &str| std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_owned());
    let die = |e: String| -> ! { eprintln!("t3box-bootctl: {e}"); std::process::exit(1) };
    let cfg = Config {
        autoboot: env("T3BOX_AUTOBOOT", "/boot/selector/autoboot.txt").into(),
        state: env("T3BOX_BOOT_STATE", "/data/t3box/update/bootctl.json").into(),
        slots: bootctl::parse_slots(&env("T3BOX_BOOT_SLOTS", "A=2,B=3")).unwrap_or_else(|e| die(e)),
        device_tree: env("T3BOX_DEVICE_TREE", "/proc/device-tree/chosen/bootloader").into(),
    };
    let socket = PathBuf::from(env("T3BOX_ENGINE_SOCKET", "/run/t3box/engine.sock"));
    let data = PathBuf::from(env("T3BOX_DATA_DIR", "/data/t3box"));
    let attempts: u32 = env("T3BOX_HEALTH_ATTEMPTS", "60").parse().unwrap_or(60);
    let ctl = BootCtl { cfg };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_else(|| die("missing argument".into()));
    let rauc = |what: &str, slot: &str| {
        let rauc = env("T3BOX_RAUC", "rauc");
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
            Ok(Decision::Nothing) => { eprintln!("t3box-bootctl: no trial pending"); Ok(()) }
            Ok(Decision::TrialFailed(slot)) => {
                eprintln!("t3box-bootctl: trial of {slot} did not reach confirmation; marking it bad");
                rauc("mark-bad", "other");
                Ok(())
            }
            Ok(Decision::CheckTrial(slot)) => match bootctl::health(&socket, &data, attempts, std::time::Duration::from_secs(1)) {
                Ok(()) => { eprintln!("t3box-bootctl: {slot} healthy; committing"); rauc("mark-good", "booted"); Ok(()) }
                Err(e) => {
                    eprintln!("t3box-bootctl: {slot} unhealthy ({e}); rolling back");
                    rauc("mark-bad", "booted");
                    let reboot = env("T3BOX_REBOOT", "systemctl reboot");
                    let mut parts = reboot.split_whitespace();
                    let _ = std::process::Command::new(parts.next().unwrap_or("true")).args(parts).status();
                    Err("trial failed health checks".into())
                }
            },
            Err(e) => Err(e),
        },
        _ => {
            eprintln!("usage: t3box-bootctl get-primary|set-primary <s>|get-state <s>|set-state <s> good|bad|get-current|confirm|guard|health");
            std::process::exit(2);
        }
    };
    if let Err(e) = result { die(e); }
}

#[cfg(not(unix))]
fn main() { std::process::exit(2); }
