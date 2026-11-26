# systemd units (reference OS)

| File | Install path |
|---|---|
| `t3box.service` | `/etc/systemd/system/t3box.service` — media engine, owns `/run/t3box/engine.sock` |
| `t3box-control.service` | `/etc/systemd/system/t3box-control.service` — management plane, code in `/opt/t3box/control` |
| `t3box-provision.service` | `/etc/systemd/system/t3box-provision.service` — provisioning core on `/run/t3box-provision/provisioning.sock` |
| `t3box-provision.polkit.rules` | `/etc/polkit-1/rules.d/50-t3box-provision.rules` |
| `t3box-ble.service` | `/etc/systemd/system/t3box-ble.service` — BlueZ GATT adapter; `BindsTo=bluetooth.service`, `Restart=always` |
| `t3box-ble.dbus.conf` | `/etc/dbus-1/system.d/t3box-ble.conf` |
| `../os/bluez/main.conf` | `/etc/bluetooth/main.conf` (LE-only, static address, MTU 517) |
| `t3box.sysusers.conf` | `/usr/lib/sysusers.d/t3box.conf` |

```bash
sudo systemd-sysusers
sudo install -d -m 0755 /etc/t3box
printf 'T3BOX_CONTROL_TOKEN=%s\n' "$(openssl rand -hex 32)" | sudo install -m 0600 /dev/stdin /etc/t3box/control.env
sudo systemctl daemon-reload && sudo systemctl enable --now t3box t3box-control
systemd-analyze verify /etc/systemd/system/t3box*.service
```
The control limits (CPUQuota 50 %, MemoryMax 128M, …) are the §9 starting experiment, not measured production values. Control depends on the engine only via `Wants=`, so restarting or crashing control never stops media; verify on hardware with `systemctl kill t3box-control` during a stream.
On the reference OS the database lives in `/var/lib/t3box/control`; the Buildroot target will relocate it to `/data/t3box/control`.
