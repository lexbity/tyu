# Cleanup status

This document records cleanup items that are known from the tracked tree.

## Stale or superseded materials

- The detailed audit and design notes under `devdocs/` are ignored in git and therefore local-only.
- Older test-rigor audit notes are superseded by the current tracked docs and should not be treated as current authority.
- Versioned original-spec filenames under `devdocs/orig-spec/` are a local documentation smell, but those files are ignored and not part of the tracked repository surface.

## Known unknowns

- The root `Arith.asm` file remains unexplained in tracked documentation and code comments.
- The purpose of any ignored `devdocs/test-rigor-audit/` or `memory/test-rigor-audit.md` material should be treated as archival unless a maintainer re-verifies it.

## Current tracked status

- `README.md`, `SETUP.md`, `DOCS.md`, `CONTRIBUTING.md`, `TROUBLESHOOTING.md`, `GLOSSARY.md`, and `LIMITATIONS.md` are the discoverable documentation surface.
- `devdocs/` and `.github/` remain intentionally ignored.

## Maintainer actions still needed

- Decide whether the unexplained `Arith.asm` should be documented or removed.
- If any ignored local audit notes are still useful, add explicit stale banners in the tracked docs that point readers to the current surface.
