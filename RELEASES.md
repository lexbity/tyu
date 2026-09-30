# Releases

Release records and the release policy (PLAN-RELEASE-1 §Q2, S4). The records
are appended here by `ci/cut-release.sh` at each cut and cross-reference the
statement band rule that makes a version policy load-bearing.

## Release record

> No release has been cut yet. `v0.1.0` is cut only when the §Q7 release
> gate is green (S9). The canonical tag form is `vMAJOR.MINOR.0` — real
> patch tags are forbidden (the commit SHA is the patch identity).

Toolchain pin for all releases: `rust-toolchain.toml`
(`nightly-2026-09-29`; a pin-date bump must be noted here).

## The release pipeline (S9 — the runbook)

1. `cargo test --workspace --release` and `bash ci-lint.sh` are green on
   `main`; `gh run list --workflow CI`/`Integration` for the release commit
   show `success`.
2. **Cut and sign on this machine** (never in CI — Q6):
   `bash ci/cut-release.sh v0.1.0` — bumps crate versions, commits
   `release: vX.Y.0`, creates the annotated SSH-signed tag, archives
   `tyu-X.Y.0.tar.gz` (+ `.sha256`, `.sig`), and opens a **DRAFT** release.
   Preview mechanics with `bash ci/cut-release.sh --dry-run v0.1.0`.
3. Push the tag: `git push origin vX.Y.0`.
4. `.github/workflows/release.yml` runs the §Q7 gate from the tagged SHA
   (verify → guards → arch matrix → port → differential → acceptance →
   artifacts) and renders the gate table + lab status in the run summary.
   It never publishes.
5. Read the draft, verify it per SECURITY.md, then publish manually
   (`gh release publish vX.Y.0`).

Branch protection on `main` (PR + green CI) and tag protection on `v*` are
runbook prerequisites (see SECURITY.md §Supply chain).

## Policy

* **MAJOR** — proof-migration release. Statement encodings may break;
  release notes MUST carry a migration map (PLAN-VERIFY-3 §Q4 item 3).
* **MINOR** — stable feature/proof release, tagged `vMAJOR.MINOR.0`.
  May add features and proofs; MUST NOT change existing statement hashes.
* **PATCH** — a git SHA, never a tag. Fixes between minors are identified
  by commit SHA and recorded in `CHANGELOG.md` under the current minor.

### The band rule (PLAN-VERIFY-3 §Q4 item 3)

Within the same `major.minor` band (plus `SEMANTICS_VERSION` and the
`tyu.stmt` major), statement hashes MUST be stable. Enforcement is double:
the statement-golden CI gate (`ci.yml` x86_64 job) proves it on every push,
and `ci/cut-release.sh` re-probes it at cut time (regen twice, strict pass,
`git diff --exit-code test-goldens/statements`). A band violation aborts the
cut — the version must be MINOR with a migration map instead.

### Signing

* Tags are annotated and SSH-signed (`gpg.format = ssh`).
* The public key is committed at `docs/release-signing-key.asc`.
* Fingerprint: `SHA256:b90QFv2z8+MccQj3+IvxSRS9ITKA9XmxZ3sD+y7OmDc`
* Signatures are produced on the maintainer's machine, never in CI (Q6).
  CI validates artifact well-formedness only.
* Consumer verification is **SECURITY.md §Verifying a release** — the same
  `ssh-keygen -Y verify` / `checksum` / `git archive` commands the release
  gate (`ci/release-verify-artifacts.sh`) executes; the key file doubles as
  the allowed-signers file.