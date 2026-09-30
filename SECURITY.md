# Security

Security-relevant operations for the tyu toolchain and its delivery.

## Reporting vulnerabilities

- **Preferred**: GitHub security advisory
  <https://github.com/lexbity/tyu/security/advisories/new> (private disclosure,
  tracked, credit-granting).
- **Direct**: the maintainer at `liuprestin@gmail.com` (the signing principal
  — same identity that signs releases).

Please include the tyu version, the platform/target, and a minimal reproducer.
There is no bug bounty; my expectations are: a best-effort response window of a
few days, a fix on `main`, and an advisory/issue note.

## Supported versions

The **current minor release** is supported. There are no backpatch releases:
fixes land on `main` and in the next minor, identified by commit SHA in
`CHANGELOG.md` under the current minor (the SHA is the patch identity — real
patch tags are forbidden, PLAN-RELEASE-1 §Q2). Unreleased `main` is the
development surface and carries the same evidence gates.

## Supply chain and signing keys

### Release signing key

- Public key: `docs/release-signing-key.asc` (committed in-repo; valid both as
  a raw SSH public key and as an `ssh-keygen -Y` allowed-signers file).
- Fingerprint: `SHA256:b90QFv2z8+MccQj3+IvxSRS9ITKA9XmxZ3sD+y7OmDc`
- Signer principal (use with `-I`): `liuprestin@gmail.com`
- Tags are annotated and SSH-signed (`gpg.format = ssh`); source tarballs carry
  a detached SSH signature. **Signatures are produced on the maintainer's
  machine only (by `ci/cut-release.sh`) — CI never holds a private key** and
  only validates well-formedness (checksum ↔ signature ↔ tag).

### Supply-chain posture of the delivery itself

- All third-party Actions in `.github/workflows/` are pinned by commit SHA with
  the semantic tag in a comment (NFR-8), enforced by a `ci-lint.sh` grep gate.
- The Rust toolchain is a dated nightly pinned in `rust-toolchain.toml` and
  provisioned by rustup (G54 / the `pin_audit` CI job prove it).
- The Lean proof port's toolchain is pinned at
  `verification/ports/lean/lean-toolchain` (elan).
- `ci/install.sh` and `tyu doctor --fix` never use `sudo`/root (NFR-6); the
  only piped "download-and-execute" installers are rustup's and elan's official
  scripts, user-local, and SETUP.md shows the verify-first two-step form as
  recommended.

### Verifying a release (tag + tarball + checksum + signature)

Every release draft attaches these assets (§6.4): `tyu-<VER>.tar.gz`,
`tyu-<VER>.tar.gz.sha256`, `tyu-<VER>.tar.gz.sig`. The tag is `v<MAJOR>.<MINOR>.0`.
All of the following MUST succeed for a release to be trusted. The same
commands are what the release gate's `artifacts` job runs against the draft.

```sh
# 0. The committed public key is your trust anchor — verify it matches the
#    fingerprint above before anything else.
ssh-keygen -lf docs/release-signing-key.asc
# -> 256 SHA256:b90QFv2z8+MccQj3+IvxSRS9ITKA9XmxZ3sD+y7OmDc liuprestin@gmail.com (ED25519)

# 1. The tag is an annotated SSH-signed tag, verifiable against the committed
#    key (git's allowed-signers form of the same file).
git -c gpg.ssh.allowedSignersFile=docs/release-signing-key.asc tag -v v<VER>
# -> "Good signature ... liuprestin@gmail.com" (or the GitHub Verified badge)

# 2. The source tarball was archived from exactly that tag.
git archive --format=tar.gz -o tyu-<VER>.tar.gz v<VER>
sha256sum tyu-<VER>.tar.gz
# -> same hash as tyu-<VER>.tar.gz.sha256 from the release

# 3. The detached SSH signature validates the tarball against the committed
#    key. Data is read from STDIN; the namespace is `git`.
ssh-keygen -Y verify \
    -f docs/release-signing-key.asc \
    -I liuprestin@gmail.com \
    -n git \
    -s tyu-<VER>.tar.gz.sig \
    < tyu-<VER>.tar.gz
# -> "Good "git" signature for liuprestin@gmail.com with ED25519 key ..."

# 4. The published checksum file verifies the same tarball.
sha256sum -c tyu-<VER>.tar.gz.sha256
```

(OpenSSH 9.x `-Y sign` emits the signature on stdout; OpenSSH 10.x writes
`<file>.sig` itself. Verification is identical either way — `-Y verify` always
reads the signed data from stdin.)

## Key loss and recovery

The signing private key is held in the maintainer's SSH agent only
(bus-factor-1; the plan records this honestly, PLAN-RELEASE-1 §11). On loss or
compromise:

1. Post a **revocation notice** in `RELEASES.md` naming the old fingerprint and
   its last trusted use, and open a GitHub security advisory referencing it.
2. Generate a new SSH key, commit its public half as
   `docs/release-signing-key.asc` (this file keeps its git history — every
   prior state remains verifiable against the key that was current at the
   time), and record the new fingerprint in `RELEASES.md` and here.
3. Upload the new public key to GitHub → Settings → SSH and GPG keys → Signing
   so subsequent tags carry GitHub's `Verified` badge.

Releases made before an incident remain verifiable against the committed key
history; the revocation note simply says new releases use a new key.

## Security-relevant build/runtime invariants

The toolchain is fail-closed by construction: unsigned modules are rejected by
the loader; statement hashes are band-locked (PLAN-VERIFY-3 §Q4) and machine
enforced; the Lean port gate (conformance + axiom audit) blocks releases (Q7).
None of these change without a MAJOR release with a migration map (RELEASES.md).