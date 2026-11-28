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
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
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
    /// Trial that actually booted (seen by `restore-data` on its tryboot,
    /// before the control plane starts). Only such a trial can have
    /// changed persistent data; one that never booted needs no restore.
    pub trial_booted: Option<String>,
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
            trial_booted: v.get("trial_booted").and_then(Value::as_str).map(str::to_owned),
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

fn sync_dir(dir: &Path) {
    if let Ok(d) = std::fs::File::open(dir) { let _ = d.sync_all(); }
}

fn remove_if_exists(p: &Path) -> Result<(), String> {
    match std::fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

fn open_nofollow(p: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(p)
}

/// New file, mode 0600; fails on any existing entry, links included.
fn create_private(p: &Path) -> Result<std::fs::File, String> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(p)
        .map_err(|e| format!("{}: {e}", p.display()))
}

/// Directory readable by its owner only (created, or tightened if an
/// older image created it with 0755).
fn private_dir(dir: &Path) -> Result<(), String> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let m = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !m.is_dir() {
        return Err(format!("{}: not a directory", dir.display()));
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", dir.display()))
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
    format!("# Managed by stephania-bootctl (RAUC backend). Do not edit.\n[all]\ntryboot_a_b=1\nboot_partition={primary}\n[tryboot]\nboot_partition={trial}\n")
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
        write_atomic(&self.cfg.state, json!({"pending": s.pending, "bad": s.bad, "restore": s.restore, "restored_trial": s.restored_trial, "trial_booted": s.trial_booted}).to_string().as_bytes())
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
        s.trial_booted = None;
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
                s.trial_booted = None;
                self.write_autoboot(name, self.other(name)?)?;
            }
            self.save(&s)
        } else {
            if !s.bad.iter().any(|b| b == name) { s.bad.push(name.to_owned()); }
            if s.pending.as_deref() == Some(name) {
                let on_trial = self.booted().ok().flatten().as_deref() == Some(name) && self.booted_tryboot();
                let ran = on_trial || s.trial_booted.as_deref() == Some(name);
                s.pending = None;
                // A trial that booted may have migrated persistent data,
                // unless the crashed-trial path already restored it.
                s.restore = ran && s.restored_trial.as_deref() != Some(name);
                s.restored_trial = None;
                s.trial_booted = None;
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
    /// The snapshot holds credential hashes, so its directory is 0700 and
    /// the file is created 0600 before `sqlite3` writes into it.
    pub fn take(&self) -> Result<bool, String> {
        if !self.database.exists() {
            return Ok(false);
        }
        private_dir(&self.dir)?;
        let tmp = self.dir.join("control.db.new");
        remove_if_exists(&tmp)?;
        create_private(&tmp)?;
        let status = std::process::Command::new(&self.sqlite3)
            .arg(&self.database)
            .arg(format!(".backup '{}'", tmp.display()))
            .status()
            .map_err(|e| format!("{}: {e}", self.sqlite3))?;
        if !status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err("sqlite3 .backup failed".into());
        }
        let f = open_nofollow(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, self.snapshot_path()).map_err(|e| e.to_string())?;
        sync_dir(&self.dir);
        Ok(true)
    }

    /// Replaces the live database with the snapshot (control must not be
    /// running). Stale WAL/SHM files from the trial are removed first.
    ///
    /// Runs as root inside the control plane's own state directory, so it
    /// never follows a link there: the temporary file is created with
    /// O_EXCL|O_NOFOLLOW, and the result is owned like the directory (the
    /// control user) with the mode of the database it replaces, never
    /// world-accessible, so the control plane can still write it.
    pub fn restore(&self) -> Result<bool, String> {
        let snap = self.snapshot_path();
        let mut src = match open_nofollow(&snap) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(format!("{}: {e}", snap.display())),
        };
        if !src.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err(format!("{}: not a regular file", snap.display()));
        }
        let dir = self.database.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        // Owner and group from the state directory (systemd keeps it owned
        // by the control user), so an earlier root-owned restore is repaired.
        let d = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mode = match std::fs::symlink_metadata(&self.database) {
            Ok(m) if m.file_type().is_file() => (m.mode() & 0o660) | 0o600,
            _ => 0o600,
        };
        let (uid, gid) = (d.uid(), d.gid());
        for suffix in ["-wal", "-shm"] {
            remove_if_exists(&PathBuf::from(format!("{}{suffix}", self.database.display())))?;
        }
        let tmp = self.database.with_extension("restore");
        remove_if_exists(&tmp)?;
        let mut out = create_private(&tmp)?;
        let copied = std::io::copy(&mut src, &mut out)
            .and_then(|_| std::os::unix::fs::fchown(&out, Some(uid), Some(gid)))
            .and_then(|_| out.set_permissions(std::fs::Permissions::from_mode(mode)))
            .and_then(|_| out.sync_all());
        if let Err(e) = copied {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("restore {}: {e}", tmp.display()));
        }
        std::fs::rename(&tmp, &self.database).map_err(|e| e.to_string())?;
        sync_dir(dir);
        Ok(true)
    }

    pub fn discard(&self) {
        let _ = std::fs::remove_file(self.snapshot_path());
        sync_dir(&self.dir);
    }
}

impl BootCtl {
    /// Boot-time, before the control plane: restore data after a rejected
    /// or crashed trial, while booted from the committed slot. On the trial
    /// boot itself it only records that the trial ran. The snapshot is
    /// discarded once it has been put back.
    pub fn restore_data(&self, data: &DataSnapshot) -> Result<bool, String> {
        let mut s = self.state();
        let failed = match self.decide()? {
            Decision::CheckTrial(slot) => {
                if s.trial_booted.as_deref() != Some(slot.as_str()) {
                    s.trial_booted = Some(slot);
                    self.save(&s)?;
                }
                return Ok(false);
            }
            Decision::TrialFailed(slot) => Some(slot),
            Decision::Nothing => None,
        };
        // A crashed trial is restored once (until confirm clears pending),
        // and only if it booted: an install followed by a normal reboot
        // never ran the new version, so newer data is kept.
        let failed_needs_restore = failed.as_deref().is_some_and(|t| {
            s.restored_trial.as_deref() != Some(t) && s.trial_booted.as_deref() == Some(t)
        });
        if !(s.restore || failed_needs_restore) {
            return Ok(false);
        }
        if self.booted()?.as_deref() != Some(self.committed()?.as_str()) {
            return Ok(false);
        }
        let restored = data.restore()?;
        s.restore = false;
        if failed_needs_restore {
            s.restored_trial = failed;
        }
        self.save(&s)?;
        if restored {
            data.discard();
        }
        Ok(restored)
    }

    /// Drops the snapshot when nothing can need it any more: no trial is
    /// pending and no restore is due (after a commit, or after a trial
    /// that is rejected without having changed data).
    pub fn discard_snapshot_if_unneeded(&self, data: &DataSnapshot) {
        let s = self.state();
        if s.pending.is_none() && !s.restore {
            data.discard();
        }
    }
}

/// Release version `MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]` (SemVer
/// precedence; build metadata is ignored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    core: (u64, u64, u64),
    pre: Vec<String>,
}

impl Version {
    pub fn parse(v: &str) -> Option<Version> {
        let v = v.trim();
        let v = v.split_once('+').map_or(v, |(a, _)| a);
        let (core, pre) = match v.split_once('-') { Some((c, p)) => (c, Some(p)), None => (v, None) };
        let nums: Vec<u64> = core.split('.').map(|n| if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) { n.parse().ok() } else { None }).collect::<Option<_>>()?;
        let [major, minor, patch] = nums[..] else { return None };
        let pre: Vec<String> = match pre {
            Some(p) => p.split('.').map(str::to_owned).collect(),
            None => Vec::new(),
        };
        if pre.iter().any(|id| id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
            return None;
        }
        Some(Version { core: (major, minor, patch), pre })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        self.core.cmp(&other.core).then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Equal,
            (true, false) => Greater,
            (false, true) => Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&other.pre) {
                    let o = match (a.parse::<u64>(), b.parse::<u64>()) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        (Ok(_), Err(_)) => Less,
                        (Err(_), Ok(_)) => Greater,
                        (Err(_), Err(_)) => a.cmp(b),
                    };
                    if o != Equal { return o; }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        })
    }
}

/// `version=` from the `[update]` section of a RAUC bundle manifest.
pub fn manifest_version(manifest: &str) -> Option<String> {
    let mut section = String::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_owned();
        } else if section == "update" {
            if let Some(v) = line.strip_prefix("version").map(str::trim_start).and_then(|r| r.strip_prefix('=')) {
                return Some(v.trim().to_owned());
            }
        }
    }
    None
}

/// Anti-rollback (RAUC pre-install): a bundle older than the running
/// release is refused, so a signed but vulnerable old release cannot be
/// installed again. Reinstalling the same version is allowed. A running
/// image without a release version (development and CI images) has no
/// baseline. Going back to an older release needs the installer.
pub fn check_bundle_version(installed: Option<&str>, bundle: Option<&str>) -> Result<String, String> {
    let bundle = bundle.ok_or("bundle manifest has no [update] version")?;
    let b = Version::parse(bundle).ok_or_else(|| format!("bundle version {bundle:?} is not a release version"))?;
    let Some(i) = installed.and_then(Version::parse) else {
        return Ok(format!("installing {bundle}; the running image has no release version ({:?}), no anti-rollback baseline", installed.unwrap_or("")));
    };
    if b < i {
        return Err(format!("refusing to install {bundle}: older than the installed {} (anti-rollback)", installed.unwrap_or_default().trim()));
    }
    Ok(format!("installing {bundle} over {}", installed.unwrap_or_default().trim()))
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
    let probe = data_dir.join(".stephania-health");
    write_atomic(&probe, b"ok").and_then(|_| std::fs::remove_file(&probe).map_err(|e| e.to_string()))
        .map_err(|e| format!("data not writable: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fx { dir: PathBuf, ctl: BootCtl }
    impl Fx {
        fn new(tag: &str) -> Fx {
            let dir = std::env::temp_dir().join(format!("stephania-bootctl-{tag}-{}", std::process::id()));
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
        assert!(!fx.dir.join("snap/control.db").exists(), "snapshot discarded after the restore");
        assert!(!c.restore_data(&data).unwrap(), "restore runs once");

        // Crashed trial: no mark-bad ran, restore is driven by the pending
        // state. restore-data runs on every boot, the trial's included.
        std::fs::write(fx.dir.join("snap/control.db"), b"v2").unwrap();
        c.set_primary("B").unwrap();
        fx.firmware_boot(true);
        assert!(!c.restore_data(&data).unwrap());
        std::fs::write(&db, b"v3").unwrap();
        fx.firmware_boot(false);
        assert!(c.restore_data(&data).unwrap());
        assert_eq!(std::fs::read(&db).unwrap(), b"v2");
        std::fs::write(&db, b"v2-new-scene").unwrap(); // control runs on A
        // confirm's mark-bad failed or power was lost before it: the next
        // boot sees the same failed trial but must not restore again.
        fx.firmware_boot(false);
        std::fs::write(fx.dir.join("snap/control.db"), b"v2").unwrap();
        assert!(!c.restore_data(&data).unwrap(), "crashed trial restored only once");
        assert_eq!(std::fs::read(&db).unwrap(), b"v2-new-scene");
        c.set_state("B", false).unwrap(); // confirm: mark-bad other
        c.discard_snapshot_if_unneeded(&data);
        fx.firmware_boot(false);
        assert!(!c.restore_data(&data).unwrap(), "no second restore over newer data");
        assert_eq!(std::fs::read(&db).unwrap(), b"v2-new-scene");
        assert!(!fx.dir.join("snap/control.db").exists());
        std::fs::write(fx.dir.join("snap/control.db"), b"v2-new-scene").unwrap();

        // Successful trial: data kept, snapshot discarded after commit.
        c.set_primary("B").unwrap();
        fx.firmware_boot(true);
        assert!(!c.restore_data(&data).unwrap());
        std::fs::write(&db, b"v3").unwrap();
        c.set_state("B", true).unwrap();
        c.discard_snapshot_if_unneeded(&data);
        fx.firmware_boot(false);
        assert!(!c.restore_data(&data).unwrap());
        assert_eq!(std::fs::read(&db).unwrap(), b"v3");
        assert!(!fx.dir.join("snap/control.db").exists());
    }

    #[test]
    fn a_trial_that_never_booted_keeps_newer_data() {
        let fx = Fx::new("noboot");
        let c = &fx.ctl;
        let db = fx.dir.join("control.db");
        std::fs::create_dir_all(fx.dir.join("snap")).unwrap();
        std::fs::write(fx.dir.join("snap/control.db"), b"old").unwrap();
        let data = DataSnapshot { database: db.clone(), dir: fx.dir.join("snap"), sqlite3: "sqlite3".into() };
        c.set_primary("B").unwrap();
        // Installed, then a normal reboot (no tryboot): A boots again.
        std::fs::write(&db, b"newer").unwrap();
        fx.firmware_boot(false);
        assert_eq!(c.decide().unwrap(), Decision::TrialFailed("B".into()));
        assert!(!c.restore_data(&data).unwrap(), "the new version never ran");
        c.set_state("B", false).unwrap(); // confirm: mark-bad other
        c.discard_snapshot_if_unneeded(&data);
        fx.firmware_boot(false);
        assert!(!c.restore_data(&data).unwrap());
        assert_eq!(std::fs::read(&db).unwrap(), b"newer");
        assert!(!fx.dir.join("snap/control.db").exists(), "unused snapshot discarded");
    }

    fn mode(p: &Path) -> u32 { std::fs::symlink_metadata(p).unwrap().mode() & 0o7777 }

    #[test]
    fn snapshot_is_private_and_restore_never_follows_links() {
        let fx = Fx::new("perm");
        let live = fx.dir.join("control");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::set_permissions(&live, std::fs::Permissions::from_mode(0o700)).unwrap();
        let db = live.join("control.db");
        std::fs::write(&db, b"live").unwrap();
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
        // Stand-in for `sqlite3 DB ".backup 'PATH'"`: writes into the
        // existing file, as SQLite does, so its mode is ours.
        let fake = fx.dir.join("fake-sqlite3");
        std::fs::write(&fake, "#!/bin/sh\nt=${2#.backup \\'}; t=${t%\\'}\ncat \"$1\" >\"$t\"\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let snapdir = fx.dir.join("update/snapshot");
        std::fs::create_dir_all(&snapdir).unwrap();
        std::fs::set_permissions(&snapdir, std::fs::Permissions::from_mode(0o755)).unwrap(); // older image
        let data = DataSnapshot { database: db.clone(), dir: snapdir.clone(), sqlite3: fake.display().to_string() };
        // Another test thread forking while the script was being written
        // can make the first exec fail with ETXTBSY.
        let mut taken = data.take();
        for _ in 0..20 {
            if !matches!(&taken, Err(e) if e.contains("busy")) { break; }
            std::thread::sleep(std::time::Duration::from_millis(50));
            taken = data.take();
        }
        assert!(taken.unwrap());
        assert_eq!(std::fs::read(snapdir.join("control.db")).unwrap(), b"live");
        assert_eq!(mode(&snapdir), 0o700, "snapshot directory tightened");
        assert_eq!(mode(&snapdir.join("control.db")), 0o600, "snapshot not world-readable");

        // A link planted at the fixed temporary name must not be followed.
        let victim = fx.dir.join("victim");
        std::fs::write(&victim, b"precious").unwrap();
        std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&victim, live.join("control.restore")).unwrap();
        std::fs::write(&db, b"trial").unwrap();
        assert!(data.restore().unwrap());
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
        assert_eq!(mode(&victim), 0o600);
        assert_eq!(std::fs::read(&db).unwrap(), b"live");
        let m = std::fs::symlink_metadata(&db).unwrap();
        assert!(m.file_type().is_file());
        let d = std::fs::metadata(&live).unwrap();
        assert_eq!((m.uid(), m.gid()), (d.uid(), d.gid()), "owned like the state directory");
        assert_eq!(m.mode() & 0o777, 0o640, "mode kept, never world-accessible");
        assert!(!live.join("control.restore").exists());

        // The database itself replaced by a link: the link is replaced,
        // its target untouched.
        std::fs::remove_file(&db).unwrap();
        std::os::unix::fs::symlink(&victim, &db).unwrap();
        assert!(data.restore().unwrap());
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
        assert!(std::fs::symlink_metadata(&db).unwrap().file_type().is_file());
        assert_eq!(mode(&db), 0o600);

        // A snapshot that is a link is refused.
        std::fs::remove_file(snapdir.join("control.db")).unwrap();
        std::os::unix::fs::symlink(&victim, snapdir.join("control.db")).unwrap();
        assert!(data.restore().is_err());
    }

    #[test]
    fn versions_follow_semver_precedence() {
        let v = |s: &str| Version::parse(s).unwrap();
        assert!(v("0.2.0") < v("0.10.0"));
        assert!(v("1.0.0-rc.1") < v("1.0.0"));
        assert!(v("1.0.0-alpha") < v("1.0.0-alpha.1"));
        assert!(v("1.0.0-alpha.2") < v("1.0.0-alpha.10"));
        assert!(v("1.0.0-9") < v("1.0.0-alpha"));
        assert_eq!(v("1.2.3+build.7"), v("1.2.3"));
        for bad in ["", "1.2", "1.2.3.4", "v1.2.3", "ci-0123456789ab", "1.2.x", "1.2.3-", "1.2.3-a..b"] {
            assert!(Version::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn older_bundles_are_refused() {
        let m = "[update]\ncompatible=stephania-pi5\nversion=0.2.0\n[bundle]\nformat=verity\n[image.rootfs]\nfilename=rootfs.img\n";
        assert_eq!(manifest_version(m).as_deref(), Some("0.2.0"));
        assert_eq!(manifest_version("[image.rootfs]\nversion=9.9.9\n"), None, "only [update] counts");
        assert!(check_bundle_version(Some("0.3.0\n"), Some("0.2.0")).unwrap_err().contains("anti-rollback"));
        assert!(check_bundle_version(Some("1.0.0"), Some("1.0.0-rc.1")).is_err());
        assert!(check_bundle_version(Some("0.2.0\n"), Some("0.2.0")).is_ok(), "reinstall is allowed");
        assert!(check_bundle_version(Some("0.2.0"), Some("0.3.0")).is_ok());
        assert!(check_bundle_version(Some("0.2.0"), None).is_err());
        assert!(check_bundle_version(Some("0.2.0"), Some("ci-0123")).is_err(), "unversioned bundles are refused");
        assert!(check_bundle_version(Some("0.0.0-dev"), Some("0.1.0")).is_ok());
        assert!(check_bundle_version(Some("ci-0123456789ab"), Some("0.1.0")).is_ok(), "CI images have no baseline");
        assert!(check_bundle_version(None, Some("0.1.0")).is_ok());
    }

    #[test]
    fn autoboot_parser_ignores_other_sections() {
        assert_eq!(parse_autoboot("[pi4]\nboot_partition=7\n[all]\nboot_partition=2\n[tryboot]\nboot_partition=3"), (Some(2), Some(3)));
    }
}
