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


## Reporting vulnerabilities

- **Preferred**: GitHub security advisory
  <https://github.com/lexbity/tyu/security/advisories/new> (private disclosure,
  tracked, credit-granting).

Please include the tyu version, the platform/target, and a minimal reproducer.
There is no bug bounty; my expectations are: a best-effort response window of a
few days, a fix on `main`, and an advisory/issue note.
