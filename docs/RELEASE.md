# Releases

A tag `v<version>` on a commit of `main` runs `.github/workflows/release.yml`
and creates a **draft** GitHub release with the §7 artifacts:

| Artifact | What it is |
|---|---|
| `stephania-os-pi5.zip` | extract onto a FAT32 card; RAM installer (docs/INSTALL.md) |
| `stephania-os-pi5.img.xz` | raw A/B image for recovery flashing |
| `stephania-os-pi5.raucb` | signed verity update bundle (docs/UPDATE.md) |
| `sbom.spdx.json`, `THIRD_PARTY.md`, `LICENSES.tar.gz` | inventory and licence texts incl. Debian / Raspberry Pi copyright files (docs/LICENSING.md) |
| `stephctl-<version>-<target>.tar.gz` / `.zip` | maintenance CLI (docs/MAINTENANCE.md) for Linux, macOS and Windows |
| `SHA256SUMS`, `SHA256SUMS.sig`, `release-signing.pub` | hashes of everything above, Ed25519-signed |

## Release keys

The trust anchors are committed in this repository, independent of any
release download:

| File | What it is | SHA-256 |
|---|---|---|
| `os/release/keys/release-signing.pub` | Ed25519 key that signs `SHA256SUMS` and the installer manifest; built into the installer | file `ae3575735020476cc1a52c0dd8c7f0b8abf58ac57834f5c3714d4b1e232dc6a8` |
| `os/release/keys/rauc-ca.pem` | release CA (`CN=Stephania Release CA`, valid to 2046); the image's RAUC keyring | certificate fingerprint `78:86:00:09:78:C1:EE:15:C7:5A:2B:06:51:50:39:09:DB:AE:4A:1F:0B:6E:FA:A7:BA:62:A7:14:46:2F:87:53` |

The key fingerprint is also shown on the download page (site/download.js).
Check it with `sha256sum release-signing.pub` and
`openssl x509 -in rauc-ca.pem -noout -fingerprint -sha256`. A key that
comes with a download proves nothing on its own: anyone who rebuilds a ZIP
can put their own key next to it.

The installer's on-device signature check (docs/INSTALL.md) protects the
card against corruption and mixed-up files. It does not prove authenticity:
the key it checks against travels in the same ZIP. Authenticity rests on
verifying the download as below before writing the card.

## Verifying a download

Put the release files in one directory and run, from a checkout of this
repository:

```sh
os/release/verify.sh <dir>
```

It uses the committed keys and refuses a trusted key taken from `<dir>`.
It checks the `SHA256SUMS` signature and every listed hash (including the
stephctl archives present), the installer manifest signature, its payload
and installer hashes, that the installer's built-in key is the release
key, the RAUC bundle against the release CA, the SBOM and licence texts,
and the image archive. It needs OpenSSL 3, unzip, cpio, xz, python3 and
rauc; without rauc the bundle signature is skipped (its hash is still
covered by `SHA256SUMS`).

For a single file, such as a stephctl archive:

```sh
openssl pkeyutl -verify -pubin -inkey os/release/keys/release-signing.pub -rawin \
  -in SHA256SUMS -sigfile SHA256SUMS.sig
sha256sum -c --ignore-missing SHA256SUMS
```

## How the workflow signs

The run is split so that signing keys never exist where dependency code runs:

1. `prepare` validates the version and decides the mode. Production keys are
   used only for a `v*` tag whose commit is on `main` (a tag push, or
   `tag-release.yml` after a VERSION bump). A run dispatched on a branch is
   a **dry run**: it signs with throwaway keys (`os/release/test-keys.sh`),
   is not published, and its `release-dry-run` artifact is useless to
   devices.
2. `image` and `stephctl` build everything (npm, crates, gems in the image
   chroot) with a read-only token, no Actions cache, a toolchain image built
   from `ci/images/device-build.Dockerfile` in the run (never the mutable
   registry tag), and only the committed public keys. The installer card
   tree is left unsigned (`STEPHANIA_INSTALL_PUBLIC_KEY`) and
   `os/release/stage.sh` lists the stage in `STAGE.sha256`.
3. `sign` runs in the environment `release`. It installs only distribution
   tools (rauc, zip, cpio), refuses a stage whose `STAGE.sha256` digest
   differs from the one the image job reported or whose files differ from
   it, checks that the signing key matches `os/release/keys/release-signing.pub`
   and the RAUC certificate chains to `os/release/keys/rauc-ca.pem`, and runs
   `os/release/sign.sh` with the keys on a tmpfs that is removed when the
   step ends. It then verifies the result with the committed keys.
4. `publish` (the only job with `contents: write`) checks the signed
   `SHA256SUMS` once more and creates the draft release with every file in
   one call; nothing is uploaded with `--clobber`.

Signing material: `STEPHANIA_RELEASE_SIGNING_KEY`, `STEPHANIA_RAUC_SIGNING_CERT`,
`STEPHANIA_RAUC_SIGNING_KEY` (and optionally `STEPHANIA_RAUC_CA_CERT`, which
must equal the committed CA). If any is missing the production run fails
before signing; nothing unsigned is ever published. `os/release/assemble.sh`
runs stage and sign in one go; CI assembles and verifies the same artifacts
on every OS change with throwaway keys (`os/release/test-keys.sh`, job
`image-qemu-x64`), and `ci/release-sign-e2e.sh` checks that the signing half
refuses tampered input.

### Repository settings (maintainer, once)

These cannot be expressed in workflow files:

- Settings > Environments > `release`: required reviewer (the maintainer);
  deployment branches and tags limited to `main` and tags `v*`. Move the
  signing secrets above from repository secrets into this environment, so
  that a workflow run on any other ref cannot read them.
- Settings > Environments > `testflight`: the same, with `main` and tags
  `connect-v*`, holding `ASC_KEY_ID`, `ASC_ISSUER_ID`, `ASC_KEY_P8` and
  `APPLE_TEAM_ID` (.github/workflows/testflight.yml).
- Settings > Rules > Rulesets: restrict creation and deletion of tags `v*`
  and `connect-v*` to the maintainer (and the tag-release workflow).

### Key rotation

Generate new keys offline, commit the new public key / CA to
`os/release/keys/` (and the fingerprint here and in site/download.js) in a
reviewed pull request, then replace the secrets. The sign job refuses a
signing key that does not match the committed public key.

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
reproducibility of the whole image is **not** verified: the Debian / Raspberry Pi
packages are fetched at build time (the archive moves) and native gem
builds are not checked for determinism. Pin a snapshot mirror
(`STEPHANIA_IMAGE_MIRROR`) and rebuild twice to evaluate it.
