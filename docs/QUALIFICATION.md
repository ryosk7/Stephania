# Physical qualification procedures

These procedures cover what CI cannot prove (docs/MASTER-ARCHITECTURE.md
§32, docs/ACCEPTANCE.md): real Raspberry Pi 5 firmware and radios, HDMI
capture hardware, phones, venues and power. CI evidence (virtual BLE,
mac80211_hwsim, emulated tryboot, local RTMPS server) is a precondition,
not a substitute.

**Rules**
- A result is recorded only from an actual run on the listed hardware,
  with an evidence link (log archive, `tools/qualification/collect.sh`
  bundle, photo/video of the screen where relevant). No estimates.
- Every row in the results tables starts as `Not run`. `ci/check-qualification.py`
  fails CI if a row says `Pass`/`Fail` without an evidence link, or if
  a run lacks date, hardware and software revision.
- A failure is recorded as `Fail` with the evidence and an issue link;
  it is never deleted when a later run passes.
- Before each session run `tools/qualification/collect.sh <out-dir>` on
  the device and keep the archive with the session evidence.

## Hardware under test (fill per session)
| Item | Model / revision | Firmware / version |
|---|---|---|
| Raspberry Pi 5 | | bootloader EEPROM: |
| HDMI capture (UVC) | | |
| USB audio | | |
| BLE controller | on-board CYW43455 / USB dongle: | |
| Power supply / battery | | |
| Storage (SD / NVMe) | | |
| Phone(s) | | OS: |

## Procedures

### Q-BOOT: A/B update and tryboot on real firmware
Covers what `os.yml` emulates (docs/UPDATE.md).
1. Flash the reference image; boot; `rauc status` shows A booted, good.
2. `stephania-update <signed bundle>`; observe reboot via tryboot (`vcgencmd
   bootloader_config`, `/proc/device-tree/chosen/bootloader/tryboot` = 1).
3. Confirm unit commits: `autoboot.txt` `[all] boot_partition=3`; reboot
   normally; still B.
4. Install a bundle whose engine fails to start (test build); observe
   rollback to the previous slot after the trial; slot marked bad.
5. Install again and cut power (relay) during the trial boot at 5 points
   (firmware, kernel, systemd, engine start, before confirm); each boot
   afterwards must come up on the committed slot.
6. Cut power during `rauc install` writing at 10 random points.
**Pass:** every boot lands on a bootable committed slot; no manual
recovery; RAUC status consistent. Target from §32: 100 controlled power
interruptions across failure points in total (Q-BOOT + Q-POWER).

### Q-BLE: provisioning radio
For each controller (on-board CYW43455, external dongle):
1. Fresh device; one-tap claim with iOS (AccessorySetupKit and fallback) and Android
   app builds; scan; join WPA2 network; enroll LAN access.
2. Repeat 30 times per phone, alternating app restart and Bluetooth
   toggle mid-operation (must recover via replay; no duplicate tokens in
   `tokens.json`).
3. With 20 other BLE devices advertising nearby (venue simulation),
   measure time from app scan start to claim completion.
4. `systemctl restart bluetooth` during an exchange; the app must recover.
**Record:** success count / attempts, p50/p95 completion time, failures
with `journalctl -u stephania-ble -u bluetooth` excerpts.
**Pass:** 100 % eventual success without power-cycling the device.

### Q-WIFI: networks
1. WPA2-PSK, WPA2/WPA3 transition, WPA3-SAE only (profile key-mgmt `sae`;
   needs the Pi 5 Wi-Fi firmware's SAE support), open, hidden SSID, wrong
   password, AP out of range.
2. Reboot: device rejoins without the app.
3. DHCP lease change: `stephania-pki` reissues the certificate within 5 min;
   app reconnect with the same pin succeeds. A Wi-Fi drop shorter than an
   hour that returns the same address reissues nothing (no control restart).
4. An open AP with the SSID of the venue's WPA2 network: the app lists both,
   marks the open one and cannot join it; the WPA2 profile is kept.
**Pass:** correct state reported over BLE for every case; reconnection
after reboot within 60 s of AP availability.

### Q-LAN: trusted LAN TLS on phones and browsers
1. App connects to `https://<host>.local:9443` and to the IP fallback
   with CA pinning (once the pinned client exists; until then record as
   blocked).
2. Browser on a laptop with the device CA installed; verify no warning,
   name constraints visible in the certificate viewer.
3. mDNS resolution on iOS, Android, macOS, Windows on the venue network.
4. Factory reset (physical button): previously issued token rejected.
**Pass:** all connections verified against the pinned CA; revoked token
gets 401.

### Q-AV: capture, encode, stream
1. HDMI 1080p30 slides with small code font; record via RTMPS to YouTube
   (unlisted) for 3 h: HDMI-only, HDMI + rear phone, HDMI + dual phone.
2. Measure A/V sync at start, 1 h, 2 h, 3 h (clap/flash test pattern,
   frame-accurate review of the YouTube VOD).
3. Unplug/replug HDMI and USB audio during the stream; network drop of
   10 s, 60 s; projector EDID changes; presenter laptops (Windows, macOS).
4. Program transitions from Studio every 2 min for the full run.
**Record:** dropped frames (engine stats + YouTube health), sync offsets,
reconnect times, `vcgencmd measure_temp`/`get_throttled` every minute.
**Pass:** no stream end; sync within ±1 frame after each recovery;
transitions frame-exact (as `ci/composition_check.py` defines); no
throttling.

### Q-POWER: power and thermal
1. Supplied PSU and the chosen battery: boot, stream 3 h, measure current
   (USB-C meter) and temperature in a closed case at 30 °C ambient.
2. Power cut at random points (idle, streaming, saving a scene, during
   update) × 50; the data partition mounts clean, control DB intact
   (`sqlite3 control.db 'PRAGMA integrity_check'`).
**Pass:** no data loss beyond the last unacknowledged write; boots
unattended.

### Q-PHONE: phone media (docs/PHONE.md "Phone app sender")
iOS MultiCam devices (and later ≥3 Android vendors/SoCs): Stephania Connect
development build, phone on the same Wi-Fi as the Pi, HDMI streaming.
1. Rear only, then Both: record `capabilities().dualProfiles`, the
   profile used, and `GET /api/v1/status` `sources` (geometry, fps from
   `access_units`, `fresh`).
2. Hold landscape both ways: the program shows the picture upright both
   times; portrait keeps the last landscape orientation.
3. 3 h Both on charge at room temperature: log the thermal state shown,
   plan changes and `dropped`; the HDMI broadcast has no interruption.
4. Interruptions: incoming call, lock/unlock, app switch, Control Center
   camera use. Each returns to `Live` without user action within 15 s of
   the phone becoming usable again; HDMI continues throughout.
5. A second phone: refused as busy; "Take over" moves the session.
6. Revoke camera permission in Settings: the app says so, nothing else
   breaks.
7. Hardware encoding: Instruments (VideoToolbox) or energy log shows the
   hardware H.264 encoder in use.
**Pass:** all steps as described; no decoder errors on the Pi
(`stephania-phone` log) outside the interruption windows.

## Results
Add one row per run. Columns: ID, date (UTC), hardware (refer to the table
above), software (git commit / image version), result (`Not run`, `Pass`,
`Fail`, `Blocked`), evidence (link), notes.

| ID | Date | Hardware | Software | Result | Evidence | Notes |
|---|---|---|---|---|---|---|
| Q-BOOT | | | | Not run | | |
| Q-BLE (CYW43455) | | | | Not run | | |
| Q-BLE (USB dongle) | | | | Not run | | |
| Q-WIFI | | | | Not run | | |
| Q-LAN | | | | Not run | | pinned app client not implemented yet |
| Q-AV HDMI-only | | | | Not run | | |
| Q-AV HDMI+rear | | | | Not run | | requires task #14 |
| Q-AV HDMI+dual | | | | Not run | | requires task #14 |
| Q-POWER | | | | Not run | | |
| Q-PHONE iOS | | | | Not run | | |
| Q-PHONE Android | | | | Not run | | Android paused |
