# Tyu Programming Language

[![CI](https://github.com/lexbity/tyu/actions/workflows/ci.yml/badge.svg?branch=main&event=push)](https://github.com/lexbity/tyu/actions/workflows/ci.yml?query=branch%3Amain)
[![Integration](https://github.com/lexbity/tyu/actions/workflows/integration.yml/badge.svg?branch=main&event=push)](https://github.com/lexbity/tyu/actions/workflows/integration.yml?query=branch%3Amain)
[![Install verify](https://github.com/lexbity/tyu/actions/workflows/install-verify.yml/badge.svg?branch=main)](https://github.com/lexbity/tyu/actions/workflows/install-verify.yml?query=branch%3Amain)
[![Release](https://github.com/lexbity/tyu/actions/workflows/release.yml/badge.svg?event=push)](https://github.com/lexbity/tyu/actions/workflows/release.yml?query=branch%3Amain)

A small, statically typed, **concatenative**,
rigor- and proof-focused systems
language. 

## Quick Install

Install from source recomended full details in [`SETUP.md`](SETUP.md):

```bash
git clone https://github.com/lexbity/tyu
cd tyu
bash ci/install.sh                 # newest v* tag (else main); installs into ~/.tyu
export PATH="$HOME/.tyu/bin:$PATH"
tyu build ci/smoke/hello.mod --out-dir=/tmp/smoke-out
/tmp/smoke-out/image.elf; echo $?         # 0 = green: your first build + run
```

Prerequisites come in three tiers — a higher tier includes the lower one:

| Tier | Unlocks | Additional tools |
|---|---|---|
| `hosted` | build + run hosted (x86_64-linux) programs | git, a C linker, rustup (auto-installed) |
| `metal` | bare-metal codegen + QEMU runs (x86_64 / ARM / RISC-V) | fasm, binutils, `qemu-system-*`, optional cross-GCCs |
| `proof` | the Lean verification gates (`ci/port.sh`, `ci/differential.sh`) | elan + the Lean pin in `verification/ports/lean/lean-toolchain` |

## Toolchain Features

- **One-command cross-target builds** — `tyu build` emits hosted or bare-metal images (x86_64 / ARM / RISC-V). New platform targets can be defined with platform bundles
- **Module pipeline** — pack, **encrypt**, and **sign** modules
- **Emulation before deployment: QEMU test flow** — the execution-test suites build real images and run
  them under QEMU
- **GDB debugging over RSP** — a hermetic GDB remote-serial-protocol client
  (`rsp-client`) can drive and debug targets, including under QEMU.
- **Optional formal verification pipeline** — the compiler emits proof obligations and canonical statements (certifier neutral). 
  **Lean 4** is default port (`verification/ports/lean/`) — discharges them or
  reports them open. 


## Language features

Checkout our  [book](docs/book/README.md) for more details and examples.

- **Concatenative, stack-based syntax** — the stack's shape is checked at
  compile time ([ch06](docs/book/ch06-the-stack-has-a-shape.md))
- **Static types with subtypes and contracts** — Ada/SPARK-flavored safety
  ([ch03](docs/book/ch03-types.md), [ch04](docs/book/ch04-contracts.md))
- **Effects and capabilities** — hardware access is a tracked effect
  ([ch05](docs/book/ch05-effects-and-capabilities.md))
- **Regions, no GC** — explicit, region-scoped allocation
  ([ch07](docs/book/ch07-places-borrowing-ownership.md))
- **Fixed arrays in the core** (`T'N`) — collections and algorithms live in
  the stdlib, which is itself Tyu source
- **MMIO register maps and representation control** — named devices from a
  platform descriptor ([ch08](docs/book/ch08-touching-hardware.md))
- **Tasks and channels** — opt-in concurrency with the cross-context rule
  ([ch12](docs/book/ch12-tasks-and-channels.md))
- **Provable programs** — the IR op semantics are normative and
  machine-checkable; factoring style becomes proof
  ([ch14](docs/book/ch14-proving-the-program.md),
  [ch15](docs/book/ch15-the-machine-as-witness.md))

## Documents

- **The book** — *Assembling in Tyu*:  every lab
  executed against the toolchain
  ([docs/book/README.md](docs/book/README.md)); language reference in
  [Appendix A](docs/book/appendix-a-language-reference.md); toolchain setup in
  [Appendix C](docs/book/appendix-c-toolchain-setup.md)
- **Setup** — [`SETUP.md`](SETUP.md) (the CI-executed setup reference)
- **Developer documentation** —
  [docs/DEVELOPER_DOCUMENTATION.md](docs/DEVELOPER_DOCUMENTATION.md) ·
  [docs/GLOSSARY.md](docs/GLOSSARY.md)
- **Verification design** — the Lean proof port lives in
  [`verification/ports/lean/`](verification/ports/lean/README.md)
- **Contributing** — [`CONTRIBUTING.md`](CONTRIBUTING.md)

- [LICENSE](LICENSE) · [AUTHORS.md](AUTHORS.md)
