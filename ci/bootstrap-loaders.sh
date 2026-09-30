#!/usr/bin/env bash
# PLAN-RELEASE-1 S2/S3 — per-job bare-metal target + loader bootstrap.
#
# Single, identical provisioning path for every job that runs ci/guards.sh or
# a dynamic bare-metal suite. On a fresh runner nothing is installed and
# target/ is empty (the maintainer's warm machine hid this — CI's first runs
# reded on exactly these prerequisites), so this script is the one place the
# facts live instead of 11 duplicated inline steps.
#
# What it provisions, and why:
#   1. rustup targets x86_64-unknown-none + thumbv7m-none-eabi
#      (precompiled-std targets; rust-src comes from rust-toolchain.toml).
#   2. target/device-loader/signing/thumbv7m-.../libdevice_loader_archive.a
#      — ci/guards.sh G7 requires the signed ARM loader under CI, but tyu's
#      build_device_loader_staticlib only produces it during dynamic builds.
#   3. The RISC-V riscv32im loader built with `-Z build-std=core,alloc`
#      ONLINE: tyu's loader build runs `--offline`, and the dated nightly's
#      std-source lockfile references versions a fresh runner cache lacks
#      (S3 removal trial, 2026-09-30). Building online here warms the registry
#      cache so the runner's own `--offline` build-std resolves. rust-src
#      (declared in rust-toolchain.toml) stays load-bearing for build-std.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

msg() { echo "bootstrap: $*"; }

# --- 0. workspace bar components on the pinned toolchain -------------------
# A runner's rustup auto-provisions the rust-toolchain.toml pin under ITS OWN
# profile — GitHub runners set `rustup set profile minimal`, which omits
# rustfmt and clippy. The workspace bar (G28 fmt / G29 clippy) then fails
# with "'cargo-fmt' is not installed" (S9 audit finding). Provision the bar
# explicitly. Network steps — retried per ci/retry.sh.
bash "$ROOT/ci/retry.sh" -- rustup component add rustfmt clippy

# --- 1. precompiled-std bare-metal targets ---------------------------------
# network step — retried per ci/retry.sh (build steps below stay fail-closed)
bash "$ROOT/ci/retry.sh" -- rustup target add x86_64-unknown-none thumbv7m-none-eabi

# --- 2. signed ARM loader archive (ci/guards.sh G7 prerequisite) -----------
msg "building signed ARM (thumbv7m) loader archive"
CARGO_TARGET_DIR="$ROOT/target/device-loader/signing" \
  cargo build --locked \
    --manifest-path crates/device-loader-archive/Cargo.toml \
    --target thumbv7m-none-eabi \
    --features signing --release

# --- 3. RISC-V loader via build-std (warms the --offline build-std cache) --
msg "building RISC-V (riscv32im) loader archive from std source (cache warm)"
CARGO_TARGET_DIR="$ROOT/target/device-loader/plain" \
  cargo build --locked \
    --manifest-path crates/device-loader-archive/Cargo.toml \
    --target riscv32im-unknown-none-elf \
    -Z build-std=core,alloc \
    --release

msg "bare-metal targets + loader archives provisioned"