# SETUP — building tyu from source

Tyu installs by **building from source** (there are no prebuilt binaries yet —
the moment binaries are wanted, a runtime-data bundling design is the
precondition, per the release plan's non-goals). This file is the tracked,
CI-executed setup reference: every command it gives is a command the
repository's own CI runs (the `.github/workflows` + `ci/*.sh` gates). The
README's quick start links here.

## Tiers

The toolchain is used at three increasing tiers. A higher tier includes the
lower one's prerequisites.

| Tier | What it unlocks | Additional tools |
|---|---|---|
| `hosted` | build and run hosted (x86_64-unknown-linux-gnu) programs | git, a C linker, rustup + the pinned nightly |
| `metal` | bare-metal codegen + QEMU execution (x86_64 / ARM / RISC-V) | fasm, binutils, qemu-system-x/-arm/-misc, cross toolchains |
| `proof` | the Lean verification port gates (`ci/port.sh`, `ci/differential.sh`) | elan + the Lean toolchain pin |

## Prerequisites

Blocking for the `hosted` tier — everything builds and runs with just these:

- **git** — `sudo apt-get install -y git` (Debian/Ubuntu) · `sudo dnf install -y git` (Fedora) · `sudo pacman -S git` (Arch) · `brew install git` (macOS, but see the devcontainer note below)
- **a C linker/compiler** (`cc`/`gcc`/`clang`) — `sudo apt-get install -y build-essential` · `sudo dnf groupinstall "Development Tools"` · `sudo pacman -S base-devel` · `brew install gcc`
- **rustup** (user-local; `ci/install.sh` installs it automatically when absent — the official installer, SHA-256-verified before execution):
  ```sh
  curl -fsSLO https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init
  curl -fsS https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init.sha256 | sha256sum -c -
  chmod +x ./rustup-init && ./rustup-init -y --default-toolchain none --no-modify-path
  ```
  The repository's `rust-toolchain.toml` pins a dated nightly; rustup provisions
  exactly that toolchain (plus `rust-src`) on the first build.

For the `metal` tier, add (Ubuntu/Debian package names; the CI script
`ci/bootstrap-loaders.sh` plus the job install lists are the source of
truth):

```sh
sudo apt-get install -y fasm binutils qemu-system-x qemu-system-arm qemu-system-misc \
  gcc-arm-none-eabi gcc-riscv64-unknown-elf
```

For the `proof` tier, add **elan** (the Lean version manager) and install the
pin recorded at `verification/ports/lean/lean-toolchain`:

```sh
curl -fsSL https://raw.githubusercontent.com/leanprover/elan/master/elan-init.sh -o /tmp/elan-init.sh
sh /tmp/elan-init.sh -y --default-toolchain none
~/.elan/bin/elan toolchain install "$(cat verification/ports/lean/lean-toolchain)"
```

## Install (the blessed source path)

Clone the repository, then run the installer (it re-clones the chosen ref
into `$prefix/src` and builds). **The four-line quickstart below is
byte-for-byte what the CI `install-verify` job executes in a pristine
container** — the fence IS `ci/install-verify-commands.txt`, enforced by a
ci-lint gate, so the docs and CI cannot drift:

```sh
bash ci/install.sh
export PATH="$HOME/.tyu/bin:$PATH"
tyu build ci/smoke/hello.mod --target=x86_64-unknown-linux-gnu --out-dir=/tmp/smoke-out
/tmp/smoke-out/image.elf
```

(For a bare clone, `git clone https://github.com/lexbity/tyu && cd tyu`
precedes it; the verified block starts at the installer. `ci/install.sh` is
fail-closed per stage, idempotent, and **never uses sudo**. The default ref
is the newest `v*` release tag on the origin, else the origin head, with a
visible notice. It writes `$prefix/INSTALL_RECORD` (ref, SHA, date) as its
last act. Rerun anytime — it re-clones fresh and atomically reinstalls.)

## Health checks and the first build

- `tyu doctor` is the health check (tier-scoped; `--tier=hosted|metal|proof|all`,
  `--format=human|json`; the D01–D12 catalog: D01–D03/D11/D12 hosted, D04–D09
  metal tools + platform-pack lint, D10 the Lean/elan proof tier). `--fix`
  auto-executes **only** the user-local elan bootstrap (when the proof tier is
  requested and elan is absent); every other remedy is printed as the
  platform-appropriate command (`sudo apt-get install …`, etc.) and is never
  run for you (a tool that shells out to sudo is not a tool to trust). Exit
  codes (FR-15): `0` all pass · `1` warnings only · `2` any fail · `3`
  internal error — scriptable, and echoed by the `--format=json` stream.
  `tyu toolchain check` is its per-target role-availability subset (an alias
  retained byte-for-byte, FR-22).
- The smoke contract is the IMAGE exit code (the driver's `tyu run` marker
  protocol is not the smoke signal):
  - `ci/smoke/hello.mod` — green; the built image exits `0` (the quickstart
    above builds and runs it).
  - `ci/smoke/bad.mod` — red; the built image must exit `20` (`CONTRACT_FAIL`),
    asserted by the install-verify job.

- The repository gate suite (the same commands CI runs before a merge):

  ```sh
  bash ci-lint.sh
  bash ci/guards.sh
  cargo test --workspace --release
  ```

## Building a standalone project (outside a checkout)

The toolchain is repo-anchored (`cwd/sysroot` is the default sysroot. To work
on a project that is not a tyu checkout, point the sysroot explicitly — the
resolution is `--sysroot=<dir>` (flag) → `TYU_SYSROOT` (env) → `cwd/sysroot`):

```sh
export TYU_SYSROOT="$HOME/.tyu/src/tyu/sysroot"
# or: tyu build --sysroot="$HOME/.tyu/src/tyu/sysroot" ...
```

## macOS / Windows

The **devcontainer** (`.devcontainer/`) is the blessed non-Linux path: it
provides the full `metal`/`proof` environment (fasm, binutils, QEMU,
cross-toolchains, rustup + pinned nightly, elan + pinned Lean) and is
CI-verified weekly (`container.yml`). Use it in VS Code / Codespaces.

---

Heavy suites (Lean port gate, the >=1e5 re-derivation differential) run in
CI — see `ci/port.sh`, `ci/differential.sh`, and `CONTRIBUTING.md`.