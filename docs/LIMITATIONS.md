# Known limitations

This document collects the sharp edges that are confirmed by tracked code and tests.

## Documentation governance

- `devdocs/` is ignored in git, so the detailed design notes and audits there are local-only and do not ship with the repository.
- `.github/` is also ignored in git, so the workflow file is local evidence rather than tracked governance.

## Loader trust model

- `crates/loader-core/src/platform.rs` documents a fail-open `verify_sig` default at trust Tier 0.
- Security docs must therefore state the trust tier explicitly and should not imply that signature verification is always enforced.

## Coverage gaps

- The workspace currently has 15 ignored tests. See `crates/tooling-tests/tests/langc_milestone16_tasks.rs`, `crates/tyu/tests/run_native.rs`, `crates/tyu/tests/test_matrix.rs`, `crates/tyu/tests/deploy_device.rs`, `crates/tyu/tests/deploy_fleet.rs`, `crates/tyu/tests/run_qemu_x86.rs`, and `crates/tyu/tests/deploy_hardware.rs`.
- RISC-V end-to-end coverage is thinner than x86_64 and ARM. The guide calls this out in the test-coverage discussion, and the execution suite has separate `x86_64`, `arm`, and `riscv` legs in `crates/execution-tests/Cargo.toml`.
- The effect/context matrix oracle is partially implemented by corpus and unit tests, but there is no tracked code showing an exhaustive per-cell oracle. See `crates/tooling-tests/tests/effect_corpus.rs` and `crates/tooling-tests/tests/phase17_negative_corpus.rs`.
- No fuzzing harness is tracked as part of the workspace test matrix.

## Cleanup items

- See [CLEANUP.md](CLEANUP.md) for the current cleanup status and maintainer follow-ups.

## Current policy

- Treat the tracked docs in `README.md`, `SETUP.md`, `DOCS.md`, `CONTRIBUTING.md`, `TROUBLESHOOTING.md`, `GLOSSARY.md`, and this file as the discoverable documentation surface.
- Treat everything under ignored paths as local-only working notes.
