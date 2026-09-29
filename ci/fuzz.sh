#!/usr/bin/env bash
# PLAN-VERIFY-3 P16.3 — fuzz PR-smoke (the "three decode targets, corpus
# seeds" gate, §11 criterion 8).
#
#   obl_v2_decode         fuzz target, tracked seeds ci/fuzz-seeds/obl_v2_decode/
#   verdicts_v2_decode    fuzz target, tracked seeds ci/fuzz-seeds/verdicts_v2_decode/
#   cert_index_decode     fuzz target, tracked seeds ci/fuzz-seeds/cert_index_decode/
#
# The tracked seeds are producer-faithful documents (real artifacts, verdicts
# and cert indices emitted by the codecs under test). The *live* corpus
# (`fuzz/corpus/<target>/`) is git-ignored and disposable: a fresh checkout
# gets no inputs, so this script tops it with the tracked seeds (idempotent
# `cp -n`), runs each target for the budget, and lets libFuzzer grow the
# disposable corpus freely — the committed seeds are never written to.
#
# Usage:   bash ci/fuzz.sh [--seconds N] [--target <t>]
# Exit 0 iff every target ran its budget with no crash/panic/hang.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SECONDS_BUDGET="${TYU_FUZZ_SECONDS:-30}"
CMD_ARGS=("$@")

TARGETS=(obl_v2_decode verdicts_v2_decode cert_index_decode)
FILTER=""

for i in "${!CMD_ARGS[@]}"; do
    case "${CMD_ARGS[$i]}" in
        --seconds) SECONDS_BUDGET="${CMD_ARGS[$((i+1))]}"; shift 2 2>/dev/null || true ;;
        --target)  FILTER="${CMD_ARGS[$((i+1))]}"; break ;;
    esac
done
unset CMD_ARGS

msg() { local c=$1; shift; tput setaf "$c" 2>/dev/null || true; echo "  $*"; tput sgr0 2>/dev/null || true; }

if ! command -v cargo >/dev/null 2>&1; then msg 1 "fuzz.sh: cargo missing"; exit 1; fi
if ! cargo +nightly fuzz --version >/dev/null 2>&1; then
    # Distinct skip code (42): `ci/acceptance.sh` reports CR-08 as
    # SKIP (the dedicated `fuzz` CI job runs it); the nightly is out-of-band.
    msg 3 "fuzz.sh: cargo-fuzz unavailable ('cargo +nightly fuzz --version' failed) — SKIP"
    exit 42
fi

run_one() {
    local t="$1" corpus="$ROOT/fuzz/corpus/$t" seeds="$ROOT/ci/fuzz-seeds/$t"
    mkdir -p "$corpus"
    if [ -d "$seeds" ]; then
        for f in "$seeds"/*; do
            [ -e "$f" ] || continue
            cp -n "$f" "$corpus"/ 2>/dev/null || true
        done
    fi
    local n; n="$(find "$corpus" -type f | wc -l)"
    if [ "$n" -eq 0 ]; then
        msg 1 "fuzz.sh: no seeds for $t (ci/fuzz-seeds/$t empty?)"
        return 1
    fi
    msg 2 "fuzz.sh: $t — ${n} corpus inputs (seeded + grown), ${SECONDS_BUDGET}s budget"
    ( cd "$ROOT/fuzz" && cargo +nightly fuzz run "$t" "$corpus" -- -max_total_time="$SECONDS_BUDGET" )
}

fail=0
for t in "${TARGETS[@]}"; do
    if [ -n "$FILTER" ] && [ "$FILTER" != "$t" ]; then continue; fi
    if ! run_one "$t"; then
        msg 1 "fuzz.sh: $t FAILED (crash/panic/hang or zero seeds)"
        fail=1
    fi
done

# After a clean smoke the disposable corpora contain libFuzzer's own findings
# (grew); the tracked seeds under ci/fuzz-seeds are untouched by construction.
if [ "$fail" -eq 0 ]; then
    msg 2 "fuzz.sh: FUZZ SMOKE GREEN (targets: ${TARGETS[*]}, ${SECONDS_BUDGET}s each)"
fi
exit "$fail"