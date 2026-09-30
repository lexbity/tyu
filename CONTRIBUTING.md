# Contributing to Tyu

This repository is a documentation-heavy systems workspace. The main enforcement rules live in `ci-lint.sh` and `ci/guards.sh`; this file summarizes the parts contributors are most likely to trip.

## Before you send a PR

- Run `bash ci-lint.sh` and `bash ci/guards.sh`.
- Use the right test bucket:
  - unit tests for crate-local behavior
  - `execution-tests` for QEMU-backed compile/run checks
  - `tooling-tests` for CLI and corpus behavior
- Do not hand-edit generated artifacts, especially:
  - `test-goldens/*`
  - emitted `.o`, `.lmod`, `.mod`, or `.def` files
- Keep `.def` and `.mod` sources mirrored when you change a module interface.
- Treat `abi_hash` as load-bearing: layout, ABI, and format changes are compatibility breaks, not cosmetic edits.
- If a test crate intentionally has no runnable tests, add `# guards: allow-no-tests` to that crate's `Cargo.toml` and explain why in the PR.

## Guard rules to know

- `ci-lint.sh` rejects bare `is_err()` / `is_ok()` checks that do not assert on the returned error value.
- `ci-lint.sh` rejects AEAD/AAD reimplementation in tests.
- `ci-lint.sh` rejects assertion-free test bodies.
- `ci-lint.sh` also includes a test-count guard for `tooling-tests`.
- `ci/guards.sh` fails if a crate with `#[test]` reports zero runnable tests.
- `ci/guards.sh` also fails if `test=false` or `harness=false` hides an in-source `#[test]`.

## Releases and CI (PLAN-RELEASE-1)

- Release policy is `RELEASES.md`: `vMAJOR.MINOR.0` tags, patch fixes are
  identified by commit SHA (never tagged), and the statement band rule
  (PLAN-VERIFY-3 §Q4 item 3) is what makes minor releases stable.
- Releases are cut with `ci/cut-release.sh vX.Y.0` — never by hand. The
  script refuses a cut on a dirty tree, a missing changelog section, an
  existing tag, or a previous gate that is red.
- A red CI run is a diagnostic, not a reason to remove CI: triage it
  fix-forward with `gh run view <run-id> --log-failed`, classify the
  failure (environment vs code vs workflow), fix, and push. Disabling or
  ignoring a failing workflow or job WITHOUT an inline justification
  comment is a review-blocking change (Q1).
- The toolchain is a dated nightly pin (`rust-toolchain.toml`); bumping the
  date is a normal maintenance commit that must be noted in the next
  release's notes.

## Documentation policy

- `DOCS.md` is the tracked documentation index.
- `devdocs/` is intentionally ignored in git and is not part of the versioned repository.
- Do not cite ignored local notes as repository authority in PRs or review comments.
