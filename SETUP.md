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
  curl -fsSLO https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init.sh
  sha256sum -c <(curl -fsS https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init.sh.sha256)
  sh ./rustup-init.sh -y --default-toolchain none --no-modify-path
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

Clone and run the installer (it re-clones the chosen ref into `$prefix/src`
and builds):

```sh
git clone https://github.com/lexbity/tyu
cd tyu
bash ci/install.sh              # newest v* tag, else main; prefix $HOME/.tyu
# or: bash ci/install.sh --tag v0.1.0 --prefix "$HOME/.tyu"
```

`ci/install.sh` is fail-closed per stage, idempotent, and **never uses sudo**.
The default ref is the newest `v*` release tag on the origin, falling back to
`main` with a visible notice. It writes `$prefix/INSTALL_RECORD` (ref, SHA,
date) as its last act. Rerun anytime — it re-clones fresh and atomically
reinstalls.

Then put the binaries on `PATH`:

```sh
export PATH="$HOME/.tyu/bin:$PATH"
```

## Health checks and the first build

- `tyu doctor` is the health check (tier-scoped; `--tier=hosted|metal|proof`,
  `--format=human|json`). The full catalog lands with the toolchain-health
  slice; `tyu toolchain check` is its per-target role-availability subset.
- First build = the committed smoke fixture. The smoke contract is the IMAGE
  exit code (the driver's `tyu run` marker protocol is not the smoke signal):

  ```sh
  tyu build ci/smoke/hello.mod --out-dir=/tmp/smoke-out
  /tmp/smoke-out/image.elf                  # must exit 0 (green)
  tyu build ci/smoke/bad.mod --out-dir=/tmp/smoke-bad-out
  /tmp/smoke-bad-out/image.elf              # must exit 20 (CONTRACT_FAIL)
  ```

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