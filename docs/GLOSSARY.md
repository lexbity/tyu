# Glossary

This glossary defines the terms used by the tracked docs and the compiler/runtime code.

## Word

A Tyu word is a named stack transformer with declared inputs, outputs, and optional contracts. See `crates/frontend/src/parse/decl.rs`, `crates/ir/src/lib.rs`, and `crates/semantics/src/types.rs` for the code representation.

## Module

A `.mod` file is a compilation unit. A `.def` file is the interface record used for separate compilation. See `crates/frontend/src/parse/mod.rs`, `crates/langc/src/iface.rs`, and `sysroot/Core.def`.

## Interface

An interface is the signature-only view of a module. The tracked code uses `.def` files and the interface-checking helpers in `crates/langc/src/iface.rs`.

## Effect

An effect describes what a word does to the machine or runtime model. The core effect and capability types live in `crates/ir/src/lib.rs`, `crates/ir/src/contract.rs`, and `crates/semantics/src/lib.rs`.

## Capability

A capability is a grant required by a word or module action. The tracked code treats capabilities as part of the effect/context model; see `crates/ir/src/contract.rs` and `crates/semantics/src/typecheck/context.rs`.

## Context

A context is the ambient environment used by the semantics checker when deciding whether a word is legal. See `crates/semantics/src/typecheck/context.rs` and `crates/semantics/src/typecheck/irgen/mod.rs`.

## Borrow

A borrow is an ownership-limited access to a region or root. The borrow rules are implemented across `crates/semantics/src/typecheck/` and exercised by the corpus in `crates/tooling-tests/tests/borrow_exclusivity.rs`.

## `iso`

`iso` is a region/borrowing qualifier used by the language model. The tracked semantics and tests treat it as part of the borrow/region system rather than a standalone feature.

## Region

A region is the allocation discipline used by Tyu instead of GC. See `crates/semantics/src/types.rs`, `crates/ir/src/lib.rs`, and the code generators in `crates/codegen-x86_64/src/word.rs`, `crates/codegen-arm/src/word.rs`, and `crates/codegen-riscv/src/word.rs`.

## Stack bound

`StackBound` describes the static bound on data-stack growth for a word or module. See `crates/ir/src/lib.rs`, `crates/loader-core/src/rederive.rs`, and `crates/diag-core/src/decode.rs`.

## `abi_hash`

`abi_hash` is the compatibility checksum that binds module layout, target contract, and module metadata together. It is computed in `crates/lmod/src/abi_hash.rs`, used by `crates/lmod/src/modinfo.rs`, and checked by `crates/loader-core/src/error.rs` and `crates/diag-core/src/decode.rs`.

## Trap code

A trap code is the numeric diagnostic tag attached to a runtime failure or language-emitted trap. See `crates/ir/src/lib.rs`, `crates/diag-core/src/decode.rs`, and the diagnostic corpus in `crates/tooling-tests/tests/`.

## Module format

`.lmod` is the packed module container consumed by the loader. See `crates/lmod/src/lib.rs`, `crates/lmod-pack/src/lib.rs`, and `crates/loader-core/src/load.rs`.

## Loader

The loader is the code path that authenticates, relocates, and maps a `.lmod` module. See `crates/loader-core/src/lib.rs`, `crates/loader-core/src/platform.rs`, and `crates/loader-core/src/load.rs`.
