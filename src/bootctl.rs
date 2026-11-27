//! A/B boot control for the Raspberry Pi 5 `tryboot` mechanism, exposed as
//! a RAUC custom bootloader backend (docs/MASTER-ARCHITECTURE.md §8,
//! docs/UPDATE.md).
//!
//! RAUC verifies the signed bundle and writes the inactive slot pair; this
//! module only decides which boot partition the firmware uses:
//!
//! - `autoboot.txt` on the boot-selector partition holds `[all]
//!   boot_partition=<primary>` and `[tryboot] boot_partition=<trial>`
//!   (`tryboot_a_b=1`). A `reboot "0 tryboot"` boots the trial partition
//!   exactly once; any later reboot (crash, watchdog, power loss) returns
//!   to the primary. That firmware rule is the rollback guarantee.
//! - `set-primary X` after an install does **not** switch permanently: it
//!   records X as pending and points `[tryboot]` at it.
//! - After the trial boot, `confirm` runs local health checks and calls
//!   `rauc status mark-good|mark-bad`; RAUC calls `set-state`, and only
//!   `set-state X good` while booted from X commits `[all]`.
//!
//! State (`pending`, per-slot good/bad) lives in a small JSON file on the
//! persistent data partition; all files are written atomically.
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Config {
    /// `autoboot.txt` on the boot-selector partition.
    pub autoboot: PathBuf,
    /// Persistent backend state (JSON).
    pub state: PathBuf,
    /// RAUC bootname → firmware partition number, e.g. A=2, B=3.
    pub slots: BTreeMap<String, u32>,
    /// `/proc/device-tree/chosen/bootloader` (or a test directory).
    pub device_tree: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct State {
    pub pending: Option<String>,
    pub bad: Vec<String>,
    /// Set when a trial was rejected: persistent data must be restored
    /// from the pre-install snapshot before the control plane starts.
    pub restore: bool,
    /// Trial whose data was already restored (crashed-trial path), so the
    /// later mark-bad does not restore a second time over newer writes.
    pub restored_trial: Option<String>,
}

impl State {
    fn load(path: &Path) -> State {
        let Ok(raw) = std::fs::read(path) else { return State::default() };
        let Ok(v) = serde_json::from_slice::<Value>(&raw) else { return State::default() };
        State {
            pending: v.get("pending").and_then(Value::as_str).map(str::to_owned),
            bad: v.get("bad").and_then(Value::as_array).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
            restore: v.get("restore").and_then(Value::as_bool).unwrap_or(false),
            restored_trial: v.get("restored_trial").and_then(Value::as_str).map(str::to_owned),
        }
    }
}

pub fn parse_slots(spec: &str) -> Result<BTreeMap<String, u32>, String> {
    let mut map = BTreeMap::new();
    for part in spec.split(',').filter(|p| !p.is_empty()) {
        let (name, num) = part.split_once('=').ok_or_else(|| format!("bad slot spec {part:?}"))?;
        let num: u32 = num.parse().map_err(|_| format!("bad partition in {part:?}"))?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric()) || !(1..=63).contains(&num) {
            return Err(format!("bad slot spec {part:?}"));
        }
        map.insert(name.to_owned(), num);
    }
    if map.len() != 2 {
        return Err("exactly two slots are required".into());
    }
    Ok(map)
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("new");
    let mut f = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    f.write_all(data).and_then(|_| f.sync_all()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(dir) = path.parent() {
        if let Ok(d) = std::fs::File::open(dir) { let _ = d.sync_all(); }
    }
    Ok(())
}

/// Parses `boot_partition` from the `[all]` and `[tryboot]` sections.
pub fn parse_autoboot(text: &str) -> (Option<u32>, Option<u32>) {
    let (mut section, mut all, mut tryboot) = (String::new(), None, None);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_ascii_lowercase();
        } else if let Some(v) = line.strip_prefix("boot_partition=") {
            let n = v.trim().parse().ok();
            match section.as_str() { "all" => all = n, "tryboot" => tryboot = n, _ => {} }
        }
    }
    (all, tryboot)
}

pub fn render_autoboot(primary: u32, trial: u32) -> String {
    format!("# Managed by t3box-bootctl (RAUC backend). Do not edit.\n[all]\ntryboot_a_b=1\nboot_partition={primary}\n[tryboot]\nboot_partition={trial}\n")
}

pub struct BootCtl {
    pub cfg: Config,
}

impl BootCtl {
    fn name_of(&self, partition: u32) -> Option<&str> {
        self.cfg.slots.iter().find(|(_, &p)| p == partition).map(|(n, _)| n.as_str())
    }

    fn partition(&self, name: &str) -> Result<u32, String> {
        self.cfg.slots.get(name).copied().ok_or_else(|| format!("unknown slot {name:?}"))
    }

    fn other(&self, name: &str) -> Result<&str, String> {
        self.partition(name)?;
        Ok(self.cfg.slots.keys().find(|k| k.as_str() != name).expect("two slots").as_str())
    }

    fn state(&self) -> State { State::load(&self.cfg.state) }

    fn save(&self, s: &State) -> Result<(), String> {
        write_atomic(&self.cfg.state, json!({"pending": s.pending, "bad": s.bad, "restore": s.restore, "restored_trial": s.restored_trial}).to_string().as_bytes())
    }

    /// Committed primary from `[all]` (the firmware default).
    pub fn committed(&self) -> Result<String, String> {
        let text = std::fs::read_to_string(&self.cfg.autoboot).map_err(|e| format!("{}: {e}", self.cfg.autoboot.display()))?;
        let (all, _) = parse_autoboot(&text);
        all.and_then(|p| self.name_of(p)).map(str::to_owned).ok_or_else(|| "autoboot.txt has no known [all] boot_partition".into())
    }

    fn write_autoboot(&self, primary: &str, trial: &str) -> Result<(), String> {
        write_atomic(&self.cfg.autoboot, render_autoboot(self.partition(primary)?, self.partition(trial)?).as_bytes())
    }

    /// RAUC `get-primary`: the slot the next (trial) boot will use.
    pub fn get_primary(&self) -> Result<String, String> {
        match self.state().pending {
            Some(p) => Ok(p),
            None => self.committed(),
        }
    }

    /// RAUC `set-primary`: stage a one-time trial of `name` (never a
    /// permanent switch; see module docs).
    pub fn set_primary(&self, name: &str) -> Result<(), String> {
        self.partition(name)?;
        let committed = self.committed()?;
        let mut s = self.state();
        s.bad.retain(|b| b != name);
        if name == committed {
            s.pending = None;
            self.save(&s)?;
            return self.write_autoboot(&committed, self.other(&committed)?);
        }
        s.pending = Some(name.to_owned());
        s.restore = false;
        s.restored_trial = None;
        // State first: if power fails between the writes, a pending entry
        // without a [tryboot] pointer only yields a harmless failed trial.
        self.save(&s)?;
        self.write_autoboot(&committed, name)
    }

    /// RAUC `get-state`.
    pub fn get_state(&self, name: &str) -> Result<&'static str, String> {
        self.partition(name)?;
        Ok(if self.state().bad.iter().any(|b| b == name) { "bad" } else { "good" })
    }

    /// RAUC `set-state`. `good` for the pending slot commits it, but only
    /// while actually booted from it; `bad` drops a pending trial.
    pub fn set_state(&self, name: &str, good: bool) -> Result<(), String> {
        self.partition(name)?;
        let mut s = self.state();
        if good {
            s.bad.retain(|b| b != name);
            if s.pending.as_deref() == Some(name) {
                if self.booted()?.as_deref() != Some(name) {
                    return Err(format!("refusing to commit {name}: not booted from it"));
                }
                s.pending = None;
                s.restore = false;
                self.write_autoboot(name, self.other(name)?)?;
            }
            self.save(&s)
        } else {
            if !s.bad.iter().any(|b| b == name) { s.bad.push(name.to_owned()); }
            if s.pending.as_deref() == Some(name) {
                s.pending = None;
                // The trial may have migrated persistent data.
                s.restore = s.restored_trial.as_deref() != Some(name);
                s.restored_trial = None;
                self.save(&s)?;
                let committed = self.committed()?;
                return self.write_autoboot(&committed, self.other(&committed)?);
            }
            self.save(&s)
        }
    }

    fn dt_u32(&self, name: &str) -> Option<u32> {
        let b = std::fs::read(self.cfg.device_tree.join(name)).ok()?;
        (b.len() == 4).then(|| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// RAUC `get-current`: slot the firmware booted (device tree).
    pub fn booted(&self) -> Result<Option<String>, String> {
        Ok(self.dt_u32("partition").and_then(|p| self.name_of(p)).map(str::to_owned))
    }

    pub fn booted_tryboot(&self) -> bool { self.dt_u32("tryboot") == Some(1) }

    /// What `confirm` must do on this boot.
    pub fn decide(&self) -> Result<Decision, String> {
        let s = self.state();
        let Some(pending) = s.pending else { return Ok(Decision::Nothing) };
        let booted = self.booted()?;
        if booted.as_deref() == Some(pending.as_str()) && self.booted_tryboot() {
            Ok(Decision::CheckTrial(pending))
        } else {
            // The firmware fell back to the primary: the trial did not
            // survive to confirmation (crash, watchdog, power loss).
            Ok(Decision::TrialFailed(pending))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Nothing,
    /// Booted the pending slot via tryboot: run health checks.
    CheckTrial(String),
    /// Booted the primary while a trial was pending.
    TrialFailed(String),
}

/// Persistent-data protection across a rejected trial (§8: restore a
/// compatible snapshot on rollback). The pre-install handler snapshots the
/// control database with SQLite's online backup; a rejected trial sets
/// `restore`, and `restore_data` (run before the control plane starts)
/// puts the snapshot back on the committed slot.
pub struct DataSnapshot {
    /// Live SQLite database (control plane).
    pub database: PathBuf,
    /// Snapshot directory on the data partition.
    pub dir: PathBuf,
    /// `sqlite3` CLI used for the online backup.
    pub sqlite3: String,
}

impl DataSnapshot {
    fn snapshot_path(&self) -> PathBuf { self.dir.join("control.db") }

    /// Consistent online copy; replaces any previous snapshot atomically.
    pub fn take(&self) -> Result<bool, String> {
        if !self.database.exists() {
            return Ok(false);
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let tmp = self.dir.join("control.db.new");
        let _ = std::fs::remove_file(&tmp);
        let status = std::process::Command::new(&self.sqlite3)
            .arg(&self.database)
            .arg(format!(".backup '{}'", tmp.display()))
            .status()
            .map_err(|e| format!("{}: {e}", self.sqlite3))?;
        if !status.success() {
            return Err("sqlite3 .backup failed".into());
        }
        let f = std::fs::File::open(&tmp).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, self.snapshot_path()).map_err(|e| e.to_string())?;
        Ok(true)
    }

    /// Replaces the live database with the snapshot (control must not be
    /// running). Stale WAL/SHM files from the trial are removed first.
    pub fn restore(&self) -> Result<bool, String> {
        let snap = self.snapshot_path();
        if !snap.exists() {
            return Ok(false);
        }
        for suffix in ["-wal", "-shm"] {
            let p = PathBuf::from(format!("{}{suffix}", self.database.display()));
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", p.display())),
            }
        }
        let tmp = self.database.with_extension("restore");
        std::fs::copy(&snap, &tmp).map_err(|e| format!("copy snapshot: {e}"))?;
        std::fs::File::open(&tmp).and_then(|f| f.sync_all()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.database).map_err(|e| e.to_string())?;
        Ok(true)
    }

    pub fn discard(&self) {
        let _ = std::fs::remove_file(self.snapshot_path());
    }
}

impl BootCtl {
    /// Boot-time, before the control plane: restore data after a rejected
    /// or crashed trial, while booted from the committed slot.
    pub fn restore_data(&self, data: &DataSnapshot) -> Result<bool, String> {
        let mut s = self.state();
        let trial_failed = matches!(self.decide()?, Decision::TrialFailed(_));
        if !(s.restore || trial_failed) {
            return Ok(false);
        }
        if self.booted()?.as_deref() != Some(self.committed()?.as_str()) {
            return Ok(false);
        }
        let restored = data.restore()?;
        s.restore = false;
        if trial_failed {
            s.restored_trial = s.pending.clone();
        }
        self.save(&s)?;
        Ok(restored)
    }

    /// After a successful commit the snapshot belongs to the old version.
    pub fn discard_snapshot_if_committed(&self, data: &DataSnapshot) {
        let s = self.state();
        if s.pending.is_none() && !s.restore {
            data.discard();
        }
    }
}

/// Asks the engine for `engine.status` over its IPC socket.
#[cfg(unix)]
pub fn engine_status(socket: &Path) -> Result<Value, String> {
    use std::os::unix::net::UnixStream;
    let mut s = UnixStream::connect(socket).map_err(|e| format!("engine socket: {e}"))?;
    s.set_read_timeout(Some(crate::ipc::IO_TIMEOUT)).ok();
    s.set_write_timeout(Some(crate::ipc::IO_TIMEOUT)).ok();
    let req = json!({"version": 1, "request_id": "bootctl", "method": "engine.status"}).to_string();
    crate::ipc::write_frame(&mut s, req.as_bytes()).map_err(|e| e.to_string())?;
    let frame = crate::ipc::read_frame(&mut s).map_err(|e| format!("{e:?}"))?.ok_or("engine closed the connection")?;
    let v: Value = serde_json::from_slice(&frame).map_err(|e| e.to_string())?;
    if v.get("ok") != Some(&Value::Bool(true)) {
        return Err(format!("engine error: {}", v["error"]));
    }
    Ok(v["result"].clone())
}

/// Updates must never start while a stream is requested (§8).
#[cfg(unix)]
pub fn install_guard(socket: &Path) -> Result<(), String> {
    match engine_status(socket) {
        Ok(st) if st.get("streaming_requested") == Some(&Value::Bool(false)) => Ok(()),
        Ok(_) => Err("refusing to update while streaming".into()),
        // No engine means nothing can be on air; the update may proceed.
        Err(e) if e.starts_with("engine socket:") => Ok(()),
        Err(e) => Err(e),
    }
}

/// Local health only (no network, no HDMI): engine answers IPC and the
/// persistent data directory is writable.
#[cfg(unix)]
pub fn health(socket: &Path, data_dir: &Path, attempts: u32, delay: std::time::Duration) -> Result<(), String> {
    let mut last = String::new();
    for i in 0..attempts.max(1) {
        match engine_status(socket) {
            Ok(_) => { last.clear(); break; }
            Err(e) => last = e,
        }
        if i + 1 < attempts { std::thread::sleep(delay); }
    }
    if !last.is_empty() { return Err(last); }
    let probe = data_dir.join(".t3box-health");
    write_atomic(&probe, b"ok").and_then(|_| std::fs::remove_file(&probe).map_err(|e| e.to_string()))
        .map_err(|e| format!("data not writable: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fx { dir: PathBuf, ctl: BootCtl }
    impl Fx {
        fn new(tag: &str) -> Fx {
            let dir = std::env::temp_dir().join(format!("t3box-bootctl-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("dt")).unwrap();
            std::fs::write(dir.join("autoboot.txt"), render_autoboot(2, 3)).unwrap();
            let cfg = Config { autoboot: dir.join("autoboot.txt"), state: dir.join("state.json"), slots: parse_slots("A=2,B=3").unwrap(), device_tree: dir.join("dt") };
            let fx = Fx { dir, ctl: BootCtl { cfg } };
            fx.boot(2, false);
            fx
        }
        /// Emulates the Pi firmware: tryboot uses [tryboot] once.
        fn boot(&self, partition: u32, tryboot: bool) {
            std::fs::write(self.dir.join("dt/partition"), partition.to_be_bytes()).unwrap();
            std::fs::write(self.dir.join("dt/tryboot"), (tryboot as u32).to_be_bytes()).unwrap();
        }
        fn firmware_boot(&self, tryboot: bool) {
            let (all, trial) = parse_autoboot(&std::fs::read_to_string(&self.ctl.cfg.autoboot).unwrap());
            self.boot(if tryboot { trial.unwrap() } else { all.unwrap() }, tryboot);
        }
    }
    impl Drop for Fx { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.dir); } }

    #[test]
    fn successful_trial_commits_only_when_booted_from_it() {
        let fx = Fx::new("ok");
        let c = &fx.ctl;
        assert_eq!(c.get_primary().unwrap(), "A");
        c.set_state("B", false).unwrap(); // RAUC marks the target bad before writing
        c.set_primary("B").unwrap();
        assert_eq!(c.get_primary().unwrap(), "B");
        assert_eq!(c.committed().unwrap(), "A", "set-primary must not switch permanently");
        assert!(c.set_state("B", true).is_err(), "cannot commit before booting it");
        fx.firmware_boot(true);
        assert_eq!(c.booted().unwrap().as_deref(), Some("B"));
        assert_eq!(c.decide().unwrap(), Decision::CheckTrial("B".into()));
        c.set_state("B", true).unwrap();
        assert_eq!(c.committed().unwrap(), "B");
        assert_eq!(c.decide().unwrap(), Decision::Nothing);
        fx.firmware_boot(false);
        assert_eq!(c.booted().unwrap().as_deref(), Some("B"), "normal reboot stays on the new slot");
    }

    #[test]
    fn failed_or_interrupted_trial_rolls_back() {
        let fx = Fx::new("rollback");
        let c = &fx.ctl;
        c.set_primary("B").unwrap();
        // Trial boot crashes before confirmation; the firmware's next normal
        // boot is the primary.
        fx.firmware_boot(true);
        fx.firmware_boot(false);
        assert_eq!(c.decide().unwrap(), Decision::TrialFailed("B".into()));
        c.set_state("B", false).unwrap();
        assert_eq!(c.get_state("B").unwrap(), "bad");
        assert_eq!(c.committed().unwrap(), "A");
        assert_eq!(c.get_primary().unwrap(), "A");
        assert_eq!(parse_autoboot(&std::fs::read_to_string(&c.cfg.autoboot).unwrap()), (Some(2), Some(3)));
        // Health failure during the trial: mark-bad on the booted slot.
        c.set_primary("B").unwrap();
        assert_eq!(c.get_state("B").unwrap(), "good", "a new install clears bad");
        fx.firmware_boot(true);
        c.set_state("B", false).unwrap();
        fx.firmware_boot(false);
        assert_eq!(c.booted().unwrap().as_deref(), Some("A"));
        assert_eq!(c.decide().unwrap(), Decision::Nothing);
    }

    #[test]
    fn reinstalling_the_committed_slot_clears_a_trial_and_bad_input_is_rejected() {
        let fx = Fx::new("misc");
        let c = &fx.ctl;
        c.set_primary("B").unwrap();
        c.set_primary("A").unwrap();
        assert_eq!(c.get_primary().unwrap(), "A");
        assert_eq!(c.decide().unwrap(), Decision::Nothing);
        assert!(c.set_primary("C").is_err());
        assert!(parse_slots("A=2").is_err());
        assert!(parse_slots("A=2,B=x").is_err());
        assert!(parse_slots("A=2,B=99").is_err());
        std::fs::write(&c.cfg.autoboot, "[all]\nboot_partition=9\n").unwrap();
        assert!(c.committed().is_err(), "unknown partition fails closed");
    }

    #[test]
    fn rejected_trial_restores_data_on_the_committed_slot_only() {
        let fx = Fx::new("data");
        let c = &fx.ctl;
        let db = fx.dir.join("control.db");
        std::fs::write(&db, b"v2").unwrap();
        std::fs::create_dir_all(fx.dir.join("snap")).unwrap();
        // Snapshot taken by the pre-install handler (stand-in for sqlite3 .backup).
        std::fs::write(fx.dir.join("snap/control.db"), b"v2").unwrap();
        let data = DataSnapshot { database: db.clone(), dir: fx.dir.join("snap"), sqlite3: "sqlite3".into() };
        c.set_primary("B").unwrap();
        fx.firmware_boot(true);
        std::fs::write(&db, b"v3-migrated").unwrap();
        std::fs::write(fx.dir.join("control.db-wal"), b"wal").unwrap();
        assert!(!c.restore_data(&data).unwrap(), "never restore while on the trial slot");
        c.set_state("B", false).unwrap(); // unhealthy
        assert!(!c.restore_data(&data).unwrap(), "still booted from B until the reboot");
        fx.firmware_boot(false);
        assert!(c.restore_data(&data).unwrap());
        assert_eq!(std::fs::read(&db).unwrap(), b"v2");
        assert!(!fx.dir.join("control.db-wal").exists());
        assert!(!c.restore_data(&data).unwrap(), "restore runs once");

        // Crashed trial: no mark-bad ran, restore is driven by the pending state.
        c.set_primary("B").unwrap();
        fx.firmware_boot(true);
        std::fs::write(&db, b"v3").unwrap();
        fx.firmware_boot(false);
        assert!(c.restore_data(&data).unwrap());
        assert_eq!(std::fs::read(&db).unwrap(), b"v2");
        std::fs::write(&db, b"v2-new-scene").unwrap(); // control runs on A
        c.set_state("B", false).unwrap(); // confirm: mark-bad other
        fx.firmware_boot(false);
        assert!(!c.restore_data(&data).unwrap(), "no second restore over newer data");
        assert_eq!(std::fs::read(&db).unwrap(), b"v2-new-scene");
        std::fs::write(fx.dir.join("snap/control.db"), b"v2-new-scene").unwrap();

        // Successful trial: data kept, snapshot discarded after commit.
        c.set_primary("B").unwrap();
        fx.firmware_boot(true);
        std::fs::write(&db, b"v3").unwrap();
        c.set_state("B", true).unwrap();
        c.discard_snapshot_if_committed(&data);
        fx.firmware_boot(false);
        assert!(!c.restore_data(&data).unwrap());
        assert_eq!(std::fs::read(&db).unwrap(), b"v3");
        assert!(!fx.dir.join("snap/control.db").exists());
    }

    #[test]
    fn autoboot_parser_ignores_other_sections() {
        assert_eq!(parse_autoboot("[pi4]\nboot_partition=7\n[all]\nboot_partition=2\n[tryboot]\nboot_partition=3"), (Some(2), Some(3)));
    }
}
