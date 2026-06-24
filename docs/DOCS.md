# Documentation map

This is the tracked documentation index for the repository.

## Tracked docs

- `README.md`: front door, project summary, and setup entry point
- `SETUP.md`: tracked setup and local development guide
- `TROUBLESHOOTING.md`: common failures and recovery paths
- `GLOSSARY.md`: shared terminology
- `LIMITATIONS.md`: honest gaps and cleanup items
- `CLEANUP.md`: cleanup status and maintainer follow-ups
- `CONTRIBUTING.md`: contributor workflow and guard rules
- `DEVELOPER_DOCUMENTATION.md`: deep architecture and maintainer guide
- `LICENSE`: Apache 2.0 license text
- `AUTHORS.md`: author credits

## Source and build areas that are also documentation-relevant

- `crates/`: compiler, loader, codegen, tooling, and test crates
- `runtime/`: target assembly runtime and linker scripts
- `sysroot/`: Tyu standard library sources
- `test-goldens/`: checked-in golden artifacts
- `ci-lint.sh` and `ci/guards.sh`: repository policy enforcement

## Governance note

`devdocs/` and `.github/` are intentionally ignored in git. Any files that happen to exist there in a local checkout are not part of the tracked repository and should not be treated as authoritative shipped documentation.

## What to read first

1. `README.md`
2. `CONTRIBUTING.md`
3. `ci-lint.sh`
4. `ci/guards.sh`
