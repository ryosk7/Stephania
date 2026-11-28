# Installation (release ZIP and RAM installer)

§7 flow: **download → extract → copy → insert → power on**. No network,
monitor, keyboard or terminal is needed.

1. Format a microSD card as FAT32 (most cards up to 32 GB already are).
2. Extract `stephania-os-pi5.zip` onto it. To be sure the ZIP is genuine,
   check it first with `os/release/verify.sh` against the release keys
   committed in this repository (docs/RELEASE.md): the installer's own
   signature check uses a key that ships inside the same ZIP, so it catches
   corruption and mixed-up files, not a rebuilt ZIP.
3. Insert the card into the Stephania device and power it on.

The firmware boots `vmlinuz` + `installer.cpio.gz` (`config.txt`); the
installer (`os/installer/stephania-install`, PID 1 of the initramfs) then:

| Stage | What happens | If power is lost right after |
|---|---|---|
| verify | Manifest signature (Ed25519, key built into the installer), file hashes, exactly one medium carrying this release's `install_id`, card capacity, RAM; everything needed is copied to RAM | card unchanged; the next boot starts over |
| partition | GPT A/B layout (os/layout.env); the selector partition gets the installer files and an install record | **unsafe window:** the next boot reports `RECOVERY: …copy the ZIP contents again` (the payload was only in RAM) |
| staged | payload copied to the data partition | next boot resumes from the staged copy |
| written | boot-a and rootfs-a written and read back against the manifest hashes | next boot resumes |
| done | data skeleton; selector keeps only `autoboot.txt` and the install record | — |

Target identification never touches other storage: a disk qualifies only
if its first partition is FAT carrying a manifest signed by the release key
with the `install_id` from `cmdline.txt` (fresh), or a Stephania selector with
the same `install_id` (resume). Zero or several candidates → refused with
nothing written. Layout: selector 64 MiB, boot A/B 256 MiB, root A/B
1 GiB, data the rest (at least 512 MiB); a card that is too small is
refused before any write.

The unsafe window lasts from the partition-table write until the payload
is staged (seconds plus the time to write ~0.4 GB); it exists because the
release files live in the FAT32 filesystem the new layout replaces. A cut
there leaves a card that boots the installer again and asks for the ZIP to
be copied again, never a half-installed system that looks fine.

Ownership is not part of the card: after the first boot, the first phone
to claim the device in Stephania Connect owns it (contracts/ble-protocol.md).

## Verified in CI (`ci/installer-e2e.sh`, QEMU amd64)
- two cards with the same ZIP: refused, both byte-identical afterwards;
- an unrelated FAT32 USB disk attached throughout: never modified;
- power cut after verify: card byte-identical;
- power cut after partition: recovery instruction on the next boot;
- power cuts after staged and after written: the next boot resumes and
  completes;
- the installed card has the expected GPT, selector content and setup
  secret, boots and passes the full image smoke test.

## Signing
The manifest signing key is ephemeral in CI; releases sign with the
release key (`STEPHANIA_INSTALL_SIGNING_KEY`).
