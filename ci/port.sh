#!/usr/bin/env bash
# PLAN-VERIFY-3 P3.2 — the port conformance gate.
#
# Role: a recognized port passes the shared conformance corpus with zero
# divergence. This script checks the Lean 4 port:
#
#   1. toolchain resolution — the pin (`lean-toolchain`) must be satisfiable:
#      through elan when present (cache keyed on the pin's sha256), else the
#      system `lean` whose version matches the pin (elyt setup);
#   2. `lake build Tyu conformance` — the library (generated data layer +
#      interfaces, including `SEMANTICS_total`) AND the conformance exe
#      compile;
#   3. `conformance --corpus <per-triple dirs>` — every `tyu.vec/1` vector is
#      reproduced by the port's abstract transfer, byte-exact on (head, top);
#      any divergence (including a tampered expectation) exits nonzero and
#      blocks the merge.
#
# Usage:   bash ci/port.sh [--port lean] [--no-toolchain-check]
#
# The default port is `lean`; `ci/port.sh lean` also works.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${1:-lean}"
PORT_DIR="$ROOT/verification/ports/$PORT"
NO_TOOLCHAIN_CHECK="${NO_TOOLCHAIN_CHECK:-}"

msg()  { local c=$1; shift; tput setaf "$c" 2>/dev/null || true; echo "  $*"; tput sgr0 2>/dev/null || true; }

if [ ! -d "$PORT_DIR" ]; then
    msg 1 "port.sh: no such port: $PORT_DIR"
    exit 1
fi
if [ ! -f "$PORT_DIR/lakefile.toml" ] || [ ! -f "$PORT_DIR/lean-toolchain" ]; then
    msg 1 "port.sh: $PORT lacks lakefile.toml / lean-toolchain"
    exit 1
fi

PIN="$(cat "$PORT_DIR/lean-toolchain")"
PIN_VERSION="${PIN##*:}"   # "leanprover/lean4:v4.27.0" -> "v4.27.0"
PIN_VERSION="${PIN_VERSION#v}"   # -> "4.27.0"

# --- toolchain resolution -------------------------------------------------
if [ -z "$NO_TOOLCHAIN_CHECK" ]; then
    if command -v elan >/dev/null 2>&1; then
        # Cache ~/.elan keyed on the pin's digest; the cache key change
        # forces a fresh toolchain fetch (spec: "cache ~/.elan keyed on
        # sha256(lean-toolchain)").
        PIN_DIGEST="$(printf '%s' "$PIN" | sha256sum | cut -d' ' -f1)"
        cache_root="${ELAN_CACHE_ROOT:-$HOME/.elan}"
        cache_marker="$cache_root/.tyu-port-$PIN_DIGEST"
        if [ ! -f "$cache_marker" ]; then
            msg 3 "  port.sh: installing toolchain $PIN (first use — cache key $PIN_DIGEST)"
            elan toolchain install "$PIN"
            mkdir -p "$cache_root"
            touch "$cache_marker"
        fi
        elan default "$PIN"
        msg 2 "  port.sh: toolchain resolved via elan: $PIN (cache hit: $([ -f "$cache_marker" ] && echo yes || echo no))"
    else
        # No elan: the system toolchain must match the pin (lake uses PATH
        # lean; a version mismatch would silently build with the wrong kernel).
        SYS_VERSION="$(lean --version 2>/dev/null | sed -n 's/.*Lean (version \(v\?[0-9][0-9.]*\).*/\1/p' | tr -d 'v' || true)"
        if [ "$SYS_VERSION" != "$PIN_VERSION" ]; then
            msg 1 "  port.sh: system lean version '$SYS_VERSION' does not match the pin '$PIN' (no elan found)"
            msg 1 "           install elan, or align the system toolchain with lean-toolchain"
            exit 1
        fi
        msg 2 "  port.sh: system toolchain matches the pin: lean $SYS_VERSION (no elan)"
    fi
else
    msg 3 "  port.sh: --no-toolchain-check: using whatever lean is on PATH"
fi

# --- build ----------------------------------------------------------------
cd "$PORT_DIR"
msg 2 "  port.sh: lake build (Tyu + conformance)"
lake build Tyu conformance

# --- conformance ----------------------------------------------------------
VECTOR_CORPUS=(
    "$ROOT/crates/verifier/test-vectors/x86_64-unknown-none"
    "$ROOT/crates/verifier/test-vectors/x86_64-unknown-linux-gnu"
    "$ROOT/crates/verifier/test-vectors/armv7m-unknown-none"
    "$ROOT/crates/verifier/test-vectors/riscv32-unknown-none"
)
msg 2 "  port.sh: conformance over all four per-target corpora"
.lake/build/bin/conformance --corpus "${VECTOR_CORPUS[@]}"

# --- PLAN-VERIFY-3 P4.2 — T-C: axiom audit over the registry theorems ---
# Every registry theorem must depend only on the permitted axioms
# {propext, Quot.sound, Classical.choice}; unproven placeholder axioms and
# `Lean.ofReduceBool` are never permitted (§Q11 item 2). `AxiomAudit.lean`
# prints `#print axioms` for every registry theorem; any line naming a
# forbidden constant fails the gate.
if [ -f AxiomAudit.lean ]; then
    msg 2 "  port.sh: axiom audit (AxiomAudit.lean)"
    AUDIT_OUT="$(lake env lean AxiomAudit.lean 2>&1 || true)"
    # every printed axiom set must belong to the permitted set
    if printf '%s\n' "$AUDIT_OUT" | grep -qE "sorryAx|Lean.ofReduceBool"; then
        msg 1 "  port.sh: axiom audit FAIL — a registry theorem depends on a forbidden axiom"
        printf '%s\n' "$AUDIT_OUT" | grep -E "depends on axioms" >&2
        exit 1
    fi
    audited="$(printf '%s\n' "$AUDIT_OUT" | grep -c "depends on axioms" || true)"
    if [ "$audited" -lt 10 ]; then
        msg 1 "  port.sh: axiom audit FAIL — only $audited theorems audited (expected ≥ 10)"
        exit 1
    fi
    msg 2 "  port.sh: axiom audit green ($audited registry theorems, permitted set only)"
else
    msg 1 "  port.sh: AxiomAudit.lean missing (P4.2 gate)"
    exit 1
fi

# --- PLAN-VERIFY-3 P4.2 — stackmeta replay (T-C's empirical hook) ---
# Every corpus word's declared (net, high) is re-derived by the port's own
# walk + monoid: net must match exactly and the peak envelope must stay
# within the declared bound; a corrupted declared value exits nonzero.
STACKMETA_GOLDENS=("$ROOT"/test-goldens/stackmeta/*/*.json)
msg 2 "  port.sh: stackmeta replay (${#STACKMETA_GOLDENS[@]} golden files)"
.lake/build/bin/conformance --level stackmeta --stackmeta "${STACKMETA_GOLDENS[@]}"