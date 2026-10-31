#!/usr/bin/env bash
# CI rehearsal — the ubuntu-26.04 baseline, container-proven on demand.
#
# Rebuilds the .github/workflows/ci.yml WASM-job provisioning and then runs
# the full ci/wasm.sh gate suite inside a PRISTINE ubuntu:26.04 container:
#
#   - a fresh checkout of the working tree (the tree minus target/,
#     node_modules and web/pkg — so uncommitted CI changes are rehearsed)
#   - cold rustup resolving the dated nightly from rust-toolchain.toml
#   - cold crates.io / npm / playwright downloads
#   - the exact pinned CLIs from the workflow (wasm-bindgen-cli 0.2.108,
#     wasm-tools 1.262.0, wasmtime-cli 50.0.0)
#
# If the workflow provisioning is incomplete (a missing apt package, a
# nonexistent version pin, an unreachable step), this fails the same way the
# runner would — before the change is pushed. It runs every ci/wasm.sh mode
# (run, --selftest, --host-only, --host-only --selftest).
#
# Usage (requires docker; runnable from any working directory):
#   bash ci/docker-audit.sh
#
# The rehearsal container survives failed attempts for post-mortem (its gate
# logs are /tmp/g5*.log inside the container). Provisioning is idempotent,
# so a rerun costs seconds, not the full build. The container is removed
# only after a fully green run — a failed attempt's leftover container is
# reclaimed by `bash ci/clean.sh --quick` along with the gate logs.
set -euo pipefail

NAME=tyu-audit
SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TUID=$(id -u)
TGID=$(id -g)

# Reuse a still-running rehearsal container from a previous attempt; only
# recreate when absent or stopped.
if [ "$(docker container inspect -f '{{.State.Running}}' "$NAME" 2>/dev/null || true)" != "true" ]; then
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker run --name "$NAME" -d \
        -v "$SRC":/mnt/src:ro \
        -w /mnt \
        ubuntu:26.04 sleep infinity >/dev/null
fi

# On success (and only then) the container goes away.
cleanup() { docker rm -f "$NAME" >/dev/null 2>&1; }

docker exec "$NAME" bash -euc '
set -euxo pipefail
export DEBIAN_FRONTEND=noninteractive

# --- system deps: the wasm-job apt line (brotli fasm binutils) + bootstrap tooling
apt-get update -qq
apt-get install -y -qq ca-certificates curl git build-essential brotli fasm binutils

# --- node (setup-node pins 22 remotely; here >=20 is what the gates need).
# NodeSource first, distro package as fallback for unsupported codenames.
curl -fsSL https://deb.nodesource.com/setup_22.x | bash - >/dev/null \
    && apt-get install -y -qq nodejs \
    || { apt-get update -qq; apt-get install -y -qq nodejs npm; }
node --version

# --- fresh checkout of the working tree (no target/, no node_modules)
if [ ! -f /tyu/ci/wasm.sh ]; then
    rm -rf /tyu
    mkdir /tyu
    tar -C /mnt/src -cf - --exclude=./target --exclude=./web/test/node_modules \
        --exclude=./web/editor/node_modules --exclude=./web/pkg . | tar -C /tyu -xf -
fi

# --- rustup + the three cargo-installed CLIs, exactly as the workflow pins them
export RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PLAYWRIGHT_BROWSERS_PATH=/opt/pw
if [ ! -x /opt/cargo/bin/cargo ]; then
    curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile default --default-toolchain stable >/dev/null
    export PATH=/opt/cargo/bin:$PATH
    cd /tyu   # so cargo resolves the workspace rust-toolchain.toml and provisions the dated nightly
    cargo install wasm-bindgen-cli --version 0.2.108 --locked
    cargo install wasm-tools --version 1.262.0 --locked
    cargo install wasmtime-cli --version 50.0.0 --locked
else
    export PATH=/opt/cargo/bin:$PATH
fi
# The GH runner image rustup provisions the rust-toolchain.toml nightly WITH
# clippy and rustfmt (default profile); mirror that here, because the G55
# clippy gate needs the clippy component of the pinned nightly, not of stable.
rustup component add clippy 2>/dev/null || true

# --- web deps from the committed lockfiles, exactly as the workflow does
(cd /tyu/web/test && npm ci --no-fund --no-audit && npx playwright install --with-deps chromium)
(cd /tyu/web/editor && npm ci --no-fund --no-audit)

# --- a run user matching the host owner, so nothing root-owned leaks out
groupadd -g '"$TGID"' -o tyu-audit 2>/dev/null || true
useradd -u '"$TUID"' -g '"$TGID"' -o -m -d /tmp/uhome tyu-audit 2>/dev/null || true
id tyu-audit >/dev/null || { echo "docker-audit: could not create the unprivileged run user" >&2; exit 1; }
chown -R '"$TUID:$TGID"' /tyu /opt/rustup /opt/cargo /opt/pw /tmp/uhome

# --- drop to the workspace owner: the gates run unprivileged, as on a runner
exec runuser -u tyu-audit -- env HOME=/tmp/uhome \
        PATH=/opt/cargo/bin:/usr/local/bin:/usr/bin:/bin \
        RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PLAYWRIGHT_BROWSERS_PATH=/opt/pw \
    bash -euc "
        set -x
        cd /tyu
        rustc --version; cargo --version; node --version
        wasm-bindgen --version; wasm-tools --version; wasmtime --version
        bash ci/wasm.sh
        bash ci/wasm.sh --selftest
        bash ci/wasm.sh --host-only
        bash ci/wasm.sh --host-only --selftest
    "
'
echo "docker-audit: all gates green (ubuntu:26.04, fresh checkout)"
cleanup
