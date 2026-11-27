# systemd units (reference OS)

| File | Install path |
|---|---|
| `stephania.service` | `/etc/systemd/system/stephania.service` — media engine, owns `/run/stephania/engine.sock` |
| `stephania-control.service` | `/etc/systemd/system/stephania-control.service` — management plane, code in `/opt/stephania/control` |
| `stephania-provision.service` | `/etc/systemd/system/stephania-provision.service` — provisioning core on `/run/stephania-provision/provisioning.sock` |
| `stephania-provision.polkit.rules` | `/etc/polkit-1/rules.d/50-stephania-provision.rules` |
| `stephania-ble.service` | `/etc/systemd/system/stephania-ble.service` — BlueZ GATT adapter; `BindsTo=bluetooth.service`, `Restart=always` |
| `stephania-ble.dbus.conf` | `/etc/dbus-1/system.d/stephania-ble.conf` |
| `../os/bluez/main.conf` | `/etc/bluetooth/main.conf` (LE-only, static address, MTU 517) |
| `stephania.sysusers.conf` | `/usr/lib/sysusers.d/stephania.conf` |
| `stephania.tmpfiles.conf` | `/usr/lib/tmpfiles.d/stephania.conf` — shared LAN token directory |
| `stephania-pki.service` / `.timer` | `/etc/systemd/system/` — device CA and LAN TLS certificate (`stephania-pki ensure`), re-checked every 5 min; restarts control only when a certificate is reissued |

```bash
sudo systemd-sysusers
sudo install -d -m 0755 /etc/stephania
printf 'STEPHANIA_CONTROL_TOKEN=%s\n' "$(openssl rand -hex 32)" | sudo install -m 0600 /dev/stdin /etc/stephania/control.env
sudo systemd-tmpfiles --create
sudo systemctl daemon-reload && sudo systemctl enable --now stephania stephania-pki.timer stephania-control
systemd-analyze verify /etc/systemd/system/stephania*.service
```
The control limits (CPUQuota 50 %, MemoryMax 128M, …) are the §9 starting experiment, not measured production values. Control depends on the engine only via `Wants=`, so restarting or crashing control never stops media; verify on hardware with `systemctl kill stephania-control` during a stream.
On the reference OS the database lives in `/var/lib/stephania/control`; the Buildroot target will relocate it to `/data/stephania/control`.
