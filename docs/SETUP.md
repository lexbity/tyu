# Setup and local development

This document is the tracked setup reference for the repository. It is based on the tracked CI workflow and command-line parsers in the workspace. Commands below are marked `unverified` unless this pass explicitly ran them.

## Environment

- Rust stable, as pinned by `rust-toolchain.toml`
- `fasm`
- `ld`
- `qemu-system-x86_64`
- `qemu-system-arm`
- `qemu-system-riscv32`
- Optional cross toolchains:
  - `gcc-arm-none-eabi`
  - `gcc-riscv64-unknown-elf`
- `python3` for the guard script

The CI workflow installs these packages on `ubuntu-latest`; see [.github/workflows/ci.yml](.github/workflows/ci.yml) for the exact package list. That workflow file is ignored in git, so treat it as local evidence, not tracked governance.

## Common commands

The repository's top-level scripts and CI use these commands:

```bash
cargo build --release -p langc -p tyu -p lmod-pack -p lmod-encrypt -p lmod-sign  # unverified
cargo test --workspace --release  # unverified
bash ci-lint.sh  # unverified
bash ci/guards.sh  # unverified
```

If you only want to confirm the host toolchain path resolution, use:

```bash
tyu toolchain check <target>  # unverified
```

The `tyu` parser also accepts `build`, `run`, `test`, `deploy`, `toolchain`, `clean`, and `--help` / `-h`.

## First-run checks

- `TYU_BIN_DIR` is read by the execution-test helpers and should point at the directory containing built host binaries during local runs. See `crates/tyu/src/test_helpers.rs` and `crates/tooling-tests/tests/common/bin.rs`.
- `tyu test` requires `langc`, `fasm`, `ld`, and a target-specific QEMU binary for the selected triple. See `crates/tyu/src/test_cmd.rs`.
- `execution-tests` for `arm` and `riscv` spawn `qemu-system-arm` and `qemu-system-riscv32` directly. See `crates/execution-tests/tests/arm.rs` and `crates/execution-tests/tests/riscv.rs`.
- `ci/guards.sh` fails if a crate with `#[test]` reports zero runnable tests or if `test=false` / `harness=false` hides an in-source test. See `ci/guards.sh`.
- `ci-lint.sh` rejects bare `is_err()` / `is_ok()` checks that do not assert the error value, assertion-free tests, and AEAD/AAD reimplementation in tests. See `ci-lint.sh`.

## Practical startup order

1. Install the environment packages above.
2. Run `bash ci-lint.sh` and `bash ci/guards.sh`.
3. Run `cargo build --release -p langc -p tyu`.
4. Run `cargo test --workspace --release`.
5. If you are working on execution tests, set `TYU_BIN_DIR` to your release bin directory before running those suites.

## Notes

- The repository has no tracked `devdocs/` index. Use `DOCS.md` for tracked docs and treat any local `devdocs/` files as ignored workspace notes.
- This document does not claim the commands above have been executed in this session. It only records the repository-backed setup path.
