# Troubleshooting

This page records the failure modes that are visible in the tracked code and test harnesses.

## 1. Toolchain not found

If `tyu` reports `Tool not found`, the failure comes from `TyuError::ToolNotFound` in `crates/tyu/src/error.rs`.

- Use `tyu toolchain check <target>` to see how the driver resolves tools. See `crates/tyu/src/toolchain.rs`.
- The resolver checks flag overrides, then manifest entries, then environment overrides, then `PATH`. See `crates/tyu/src/toolchain.rs`.
- The `tyu` binary accepts `build`, `run`, `test`, `deploy`, `toolchain`, `clean`, and `--help` / `-h`. See `crates/tyu/src/args.rs`.

## 2. Manifest or graph failures

If `tyu` fails while reading the project manifest or resolving modules, the relevant variants are:

- `ManifestRead`
- `ManifestParse`
- `ProjectParse`
- `UnknownProfile`
- `UnknownFeature`
- `FeatureUnsupportedByTarget`
- `Graph`
- `Build`

All of these are defined in `crates/tyu/src/error.rs`.

`crates/tyu/src/project.rs` defines the manifest search and parse path, and `crates/tyu/src/graph.rs` turns module resolution failures into `TyuError::Build` messages such as circular dependency detection.

## 3. Load rejections

The loader surface uses stable numeric codes from `crates/loader-core/src/error.rs`.

Common codes include:

- `E_ABI_MISMATCH` / `LoadError::AbiMismatch` = 5200
- `E_BAD_CONTAINER` / `LoadError::BadContainer` = 5201
- `E_SIG_INVALID` / `LoadError::SigInvalid` = 5202
- `E_MODULE_DECLARES_ISR` / `LoadError::ModuleDeclaresIsr` = 5203
- `E_RELOC_UNSUPPORTED` / `LoadError::RelocUnsupported` = 5204
- `E_SYMBOL_UNRESOLVED` / `LoadError::SymbolUnresolved` = 5205
- `E_ENC_UNSUPPORTED` / `LoadError::EncUnsupported` = 5213
- `E_ENC_REQUIRES_SIGNED` / `LoadError::EncRequiresSigned` = 5214
- `E_ENC_NO_KEY` / `LoadError::EncNoKey` = 5215
- `E_ENC_AUTH_FAIL` / `LoadError::EncAuthFail` = 5216
- `E_ENC_BAD_HEADER` / `LoadError::EncBadHeader` = 5217
- `E_STACK_BOUND_UNVERIFIABLE` / `LoadError::StackBoundUnverifiable` = 5220

`crates/loader-core/src/platform.rs` documents the trust model:

- `verify_sig` defaults to trust-unconditionally at Tier 0.
- `trust_tier()` governs whether the loader expects stronger checks.
- Encryption support is optional and gated by the `encryption` feature.

That means security-sensitive docs should not imply signatures are always enforced.

## 4. Diagnostic mismatches

If a raw diagnostic is hard to interpret, use the decoder in `crates/diag-core/src/decode.rs`.

- `DiagRecord` is the wire format.
- `ModinfoIndex` resolves names from `.lang.modinfo`.
- `Diagnostic` attaches a human-readable `claim_text` to the raw `trap_code`.
- `check_abi_hash` must succeed before decode; otherwise `DecodeError::StaleMap` is returned.

Relevant code lives in `crates/diag-core/src/decode.rs` and `crates/diag-core/src/lib.rs`.

## 5. QEMU hangs and test failures

The workspace uses QEMU-backed execution tests and a separate host `tyu` runner.

- `tyu test` accepts `--timeout=<secs>` and `--runner=<mode>`; see `crates/tyu/src/args.rs`.
- `tyu test` also requires tools such as `langc`, `fasm`, `ld`, and a target QEMU binary; see `crates/tyu/src/test_cmd.rs`.
- `execution-tests` invoke `qemu-system-arm` and `qemu-system-riscv32` directly; see `crates/execution-tests/tests/arm.rs` and `crates/execution-tests/tests/riscv.rs`.
- `TYU_BIN_DIR` is used by test helpers to locate built host binaries. See `crates/tyu/src/test_helpers.rs` and `crates/tooling-tests/tests/common/bin.rs`.

If a test appears to hang, distinguish:

- missing host tool
- missing QEMU binary
- a `Runner` error from `crates/tyu/src/error.rs`
- a real guest hang that needs a shorter timeout or a different runner

## 6. Unknown / unverified

- The purpose of the root `Arith.asm` file is not established in tracked docs or code comments. Treat it as unknown until a maintainer confirms it.
