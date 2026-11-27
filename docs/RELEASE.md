# Releases

A tag `v<version>` runs `.github/workflows/release.yml` on an arm64 runner
and creates a **draft** GitHub release with the §7 artifacts:

| Artifact | What it is |
|---|---|
| `t3box-os-pi5.zip` | extract onto a FAT32 card; RAM installer (docs/INSTALL.md) |
| `t3box-os-pi5.img.xz` | raw A/B image for recovery flashing |
| `t3box-os-pi5.raucb` | signed verity update bundle (docs/UPDATE.md) |
| `sbom.spdx.json`, `THIRD_PARTY.md`, `LICENSES.tar.gz` | inventory and licence texts incl. Debian/Ubuntu copyright files (docs/LICENSING.md) |
| `SHA256SUMS`, `SHA256SUMS.sig`, `release-signing.pub` | hashes of everything above, Ed25519-signed |

Signing material exists only as repository secrets
(`T3BOX_RELEASE_SIGNING_KEY`, `T3BOX_RAUC_CA_CERT`,
`T3BOX_RAUC_SIGNING_CERT`, `T3BOX_RAUC_SIGNING_KEY`). If any is missing the
workflow fails before building; nothing unsigned is ever assembled. The
image's RAUC keyring is the release CA, so devices accept only bundles
signed under it. The same Ed25519 key signs the installer manifest (checked
by the installer built into the ZIP) and `SHA256SUMS`.

Verify a download: `os/release/verify.sh <dir> release-signing.pub <release CA>`
(SHA256SUMS signature and hashes, installer manifest signature and payload,
RAUC bundle against the keyring, SBOM content, licence texts, archive
integrity). CI assembles and verifies the same artifacts on every OS change
with throwaway keys (`os/release/test-keys.sh`, job `image-qemu-x64`).

Creating the secrets (once, offline):

```sh
openssl genpkey -algorithm ed25519 -out release-signing.pem
# RAUC: a long-lived CA (kept offline) and a code-signing certificate
# (keyUsage=digitalSignature, extendedKeyUsage=codeSigning) issued by it.
```

## Reproducibility

Inputs are pinned (Cargo.lock, package-lock.json, Gemfile.lock, the
distribution suite) and filesystems are built with fixed timestamps
(`SOURCE_DATE_EPOCH` = tag commit time): SquashFS `-mkfs-time/-all-time`,
FAT `--invariant` with a fixed volume id, ext4 with fixed UUID, hash seed
and `E2FSPROGS_FAKE_TIME`, sorted reproducible cpio and ZIP. Bit-for-bit
reproducibility of the whole image is **not** verified: the Debian/Ubuntu
packages are fetched at build time (the archive moves) and native gem
builds are not checked for determinism. Pin a snapshot mirror
(`T3BOX_IMAGE_MIRROR`) and rebuild twice to evaluate it.
