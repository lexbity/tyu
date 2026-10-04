# Appendix C — Toolchain Setup (current procedure)

> Temporary by the project's own roadmap: a script/rustup-style installer
> will replace this procedure. Until then, this is the setup this book's
> labs were verified with.

## Build the toolchain

Prerequisites: Rust (pinned by `rust-toolchain.toml`), `fasm` (flat
assembler), GNU `ld`, `cc`, and — for the QEMU track and the metal
interludes — `qemu-system-x86_64` plus the cross binutils
(`gcc-arm-none-eabi`, `gcc-riscv64-unknown-elf`). For chapter 14's proof
pipeline: **Lean 4 and lake**, matching the port's pin
(`verification/ports/lean/lean-toolchain` — v4.27.0 at draft time); with
`elan` installed the pin resolves automatically, otherwise a matching
system lean works.

```console
$ cargo build --release -p langc -p tyu -p lmod-pack -p lmod-sign -p lmod-encrypt
$ export PATH="$PWD/target/release:$PATH"     # tyu finds langc on PATH
$ langc --help
$ tyu help
```

## The two tracks

- **Hosted** (most labs): `--target=x86_64-unknown-linux-gnu` — builds and
  runs on the computer, no simulator.
- **QEMU metal** (chapters 8 and 6's interlude): `--target=x86_64-unknown-none
  --platform=x86_64-unknown-none` — runs under `qemu-system-x86_64`.

## The commands used in this book

```console
# build + direct run (hosted)
$ tyu build labs/.../lab.mod --target=x86_64-unknown-linux-gnu \
      --sysroot=sysroot --out-dir=build/<name>
$ ./build/<name>/image.elf

# harness run (checks the S marker, classifies the verdict)
$ tyu run labs/.../lab.mod --target=x86_64-unknown-linux-gnu \
      --sysroot=sysroot --out-dir=build/<name>

# MMIO chapters: name the platform pack, and use static link mode
$ tyu test --manifest=labs/ch08/manifest.toml \
      --target=x86_64-unknown-none --platform=x86_64-unknown-none \
      --mode=static

# inspection (the debugging habit of ch. 2)
$ langc --emit=tc labs/.../lab.mod --sysroot=sysroot   # per-term stack trace
$ langc --emit=ir labs/.../lab.mod --sysroot=sysroot   # IR dump
```

## A project manifest (optional)

`tyu.toml` in a project directory removes the repeated flags:

```toml
[project]
main = "src/main.mod"
modules = ["src/"]

[targets.dev]
triple = "x86_64-unknown-linux-gnu"

[profile.dev]
features = ["concurrency", "module-loading"]

[toolchain.x86_64-unknown-none]
qemu = "qemu-system-x86_64"
```

Run from the project root with `tyu run src/main.mod --profile=dev`.

## Clean state

Stale builds: delete the lab's `--out-dir` (each lab keeps its own) or
`tyu clean`. The test harness builds fixtures in fresh temporary
directories by design — two modules with the same module name sharing one
out-dir can silently reuse a neighbor's object.

## The proof pipeline (chapter 14)
$ tyu proof init --dir=. <Module>.mod          # scaffold proofs/ once
$ tyu build <Module>.mod --verify-tool=lean …  # generate + lake + harvest
$ tyu proof fill --dir=. <Module>.mod          # automation candidates
$ bash ci/port.sh lean                         # the port conformance gate

# The test suites the book quotes

```console
$ cargo test -p execution-tests --test dynamic_signed      # ch. 13, signed load
$ cargo test -p execution-tests --test dynamic_negative    # ch. 13, refusals
$ cargo test -p execution-tests --test dynamic_encrypted   # ch. 13, at rest
```
