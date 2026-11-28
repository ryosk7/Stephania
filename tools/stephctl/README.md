# stephctl

Maintenance access to a Stephania device from a laptop (docs/MAINTENANCE.md).
It wraps the system OpenSSH client; there are no keys to create or register.

```text
stephctl login                     # find the device, compare the fingerprint with Connect, sign in
stephctl ssh                       # asks for the 6-digit code from Stephania Connect
stephctl ssh -- steph status       # one command
stephctl cp bundle.raucb device:   # to the maintenance inbox
stephctl cp device:/var/log/x.log .
stephctl window --close
```

Needs an admin Studio account and the OpenSSH client (`ssh`, `scp` ≥ 8.7).

Files: `devices.toml` in `~/.config/stephctl` (`%APPDATA%\stephctl` on
Windows): names, addresses and the pinned CA, no secrets. Sessions are kept
in the OS credential store (Keychain, Credential Manager, the Linux kernel
keyring, which forgets them at reboot); where none works they go to a 0600
file under `sessions/`.

Scripting and tests:

| Variable / option | Effect |
|---|---|
| `--username`, `--password-stdin`, `STEPHCTL_USERNAME` | sign in without prompts |
| `--trust <fingerprint>` | pin only if the CA has this fingerprint |
| `--code`, `STEPHCTL_CODE` | the maintenance code (one try) |
| `STEPHCTL_CONFIG_DIR` | another config directory |
| `STEPHCTL_SESSION_STORE=file` | keep sessions in files instead of the credential store |
| `STEPHCTL_SSH_USER`, `STEPHCTL_SSH_PORT` | test only: log in as another user/port than the device says (ci/stephctl-e2e.sh) |

Exit codes: 2 device unreachable, 3 certificate does not match the pin,
4 code not accepted or window closed, 5 account cannot open maintenance,
6 OpenSSH client missing; otherwise the exit code of `ssh`/`scp`.
