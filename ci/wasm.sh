#!/usr/bin/env bash
# PLAN-WASM-1 CI gates — G52–G56 (declared here, NOT in ci/guards.sh).
#
# The existing `ci/guards.sh` registry ends at G51 (§Q10). The WASM REPL
# plan's new gates live in this file so the flat G1–G51 registry is not
# renumbered/interleaved:
#
#   G52  Fork ↔ `src_interp` fragvec corpus trace agreement            (P1)
#   G53  Interpreter ↔ native execution goldens (+IR render parity)    (P2/P6)
#   G54  codegen-wasm: wasmtime validate + run, trace parity            (P7/P8)
#   G55  Browser bundle budgets, artifact hashes, now_ms grep, smoke    (P5)
#   G56  REPL session goldens byte-exact                                (P3)
#
# Implemented: G52 (P1), G53(a) (P2: execution differential + props +
# IR-render byte-parity vs `langc --emit=ir`), G56 (P3: session goldens).
#
# Usage:
#   bash ci/wasm.sh            run every gate defined so far
#   bash ci/wasm.sh --selftest run every gate's red-path demo and require
#                              each to FAIL (AC-9 — a gate that cannot be
#                              demonstrated red is not a gate)
#
# Runs standalone on a fresh clone with only the documented toolchain (AC-1).
# `ci/guards.sh` carries a pointer comment for G52–G56 and no gate edits.

set -euo pipefail

RED=1; GREEN=2; YELLOW=3; NC=0
failures=0

msg() {
    local c=$1; shift
    tput setaf "$c" 2>/dev/null || true
    echo "  $*"
    tput sgr0 2>/dev/null || true
}

fail() {
    msg $RED "FAIL: $*"
    failures=$((failures + 1))
}

# ---------------------------------------------------------------------------
# G52 (P1): the fork (`crates/wasm-repl/src/frag.rs`) is pinned to the frozen
# `verifier::src_interp` over the ENTIRE committed `tyu.fragvec/1` corpus.
# The corpus (crates/verifier/test-vectors/fragment/index.json) is the
# rendezvous point: the fork's live run must reproduce the committed traces
# AND agree with the frozen upstream's live run on the same canonical text.
# ---------------------------------------------------------------------------

g52_green() {
    msg $YELLOW "G52: fork ↔ src_interp fragvec corpus trace agreement"

    # FR-1 / I-6 preamble: the fork must build on the host. `cargo build`
    # alone cannot detect an *added* dependency (it would build fine), so the
    # dep-graph allowlist check below is the load-bearing FR-1 gate; this
    # build proves the crate still compiles no_std on the host toolchain.
    if cargo build -p wasm-repl --release > /tmp/wasm-repl-build.log 2>&1; then
        msg $GREEN "  G52: host build of wasm-repl (no_std, forbid(unsafe_code) — FR-1)"
    else
        fail "wasm-repl host build (see /tmp/wasm-repl-build.log)"
    fi

    # FR-1 / I-6, load-bearing check: every `[dependencies]` entry in the
    # crate manifest must be an in-tree R-A-allowed crate (path deps). The
    # *transitive* closure may include those crates' own deps (e.g. verifier's
    # `sha2`) — that is the allowed crates' business, not a wasm-repl dep. A
    # NEW direct dep (even a cleanly-compiling registry crate) reds this gate.
    local bad
    bad=$(
        awk '/^\[dependencies\]/{f=1;next} /^\[/{f=0} f' crates/wasm-repl/Cargo.toml \
            | grep -E '^[a-z][a-z0-9_-]*[[:space:]]*=' \
            | cut -d= -f1 | sed 's/[[:space:]]//g' \
            | grep -vE '^(ir|frontend|semantics|verifier)$' || true
    )
    if [ -n "$bad" ]; then
        fail "wasm-repl [dependencies] has a crate outside the in-tree R-A allowlist (FR-1, I-6): $bad"
    else
        msg $GREEN "  G52: wasm-repl [dependencies] is the in-tree R-A allowlist only (FR-1, I-6)"
    fi

    # The differential itself.
    if cargo test -p wasm-repl --test frag_parity --release > /tmp/g52-parity.log 2>&1; then
        msg $GREEN "  G52: fragvec corpus differential green (fork = corpus = frozen upstream)"
    else
        fail "fragvec differential (see /tmp/g52-parity.log)"
    fi

    # FR-15 / D-2: `src_interp` is cfg-gated behind the verifier's `test-util`
    # feature, and the featureless production build of verifier contains NO
    # fragment engine symbols.
    if grep -B1 "pub mod src_interp;" crates/verifier/src/lib.rs | grep -q 'cfg(feature = "test-util")'; then
        msg $GREEN "  G52: src_interp is test-util-gated in crates/verifier/src/lib.rs (FR-15)"
    else
        fail "src_interp is not test-util-gated in crates/verifier/src/lib.rs (FR-15)"
    fi

    # The symbol-absence proof builds into an ISOLATED target dir: the shared
    # target/release/deps accumulates multiple libverifier-<hash>.rlib
    # artifacts (test-util and featureless variants coexist across builds), so
    # picking one blind can select a stale test-util rlib and spuriously fail
    # the proof. In the isolated dir every libverifier rlib is a product of
    # the featureless build below; checking ALL of them is sound (the dir is
    # only ever written by this gate). Gitignored via the `target` entry.
    if CARGO_TARGET_DIR="$PWD/target/g52-prod" cargo build -p verifier --release \
        > /tmp/verifier-prod-build.log 2>&1; then
        if ! command -v nm >/dev/null 2>&1; then
            msg $YELLOW "  G52: (nm unavailable; symbol-absence check skipped)"
        else
            local r found=0 sym_hit=0
            for r in target/g52-prod/release/deps/libverifier-*.rlib; do
                [ -f "$r" ] || continue
                found=1
                if nm "$r" 2>/dev/null | grep -q "src_interp"; then
                    sym_hit=1
                fi
            done
            if [ "$found" -eq 0 ]; then
                fail "no libverifier rlib under target/g52-prod (build layout changed?)"
            elif [ "$sym_hit" -eq 1 ]; then
                fail "featureless verifier build contains src_interp symbols (nm under target/g52-prod, FR-15)"
            else
                msg $GREEN "  G52: production 'cargo build -p verifier' contains no src_interp symbols (FR-15)"
            fi
        fi
    else
        fail "featureless production build of verifier (see /tmp/verifier-prod-build.log)"
    fi

    # The verifier's OWN corpus gates must still pass with the feature on
    # (the gating must not have orphaned the upstream tests).
    if cargo test -p verifier --features test-util \
        --test cross_surface --test fragment_vectors --release > /tmp/g52-verifier.log 2>&1; then
        msg $GREEN "  G52: upstream cross_surface + fragment_vectors green under --features test-util"
    else
        fail "upstream fragment gates under --features test-util (see /tmp/g52-verifier.log)"
    fi
}

g52_selftest() {
    # Red-path demo (AC-9): corrupt `add_i64` → `Sub` under `wasm-selftest` and
    # REQUIRE the differential to fail. A green result here means the G52 gate
    # cannot see a broken mnemonic mapping — i.e. the gate is not a gate.
    #
    # Scope note: the differential is G52's primary property, so this is the
    # red path AC-9 requires. The structural sub-checks (FR-1 dep allowlist,
    # FR-15 cfg gate + symbol absence) fail trivially on their violations (an
    # added registry crate, a removed cfg line, leaked symbols) and are
    # asserted green in run mode — not mutation-demonstrated.
    msg $YELLOW "G52 selftest: corrupted 'add_i64' mapping must red the differential"
    if cargo test -p wasm-repl --features wasm-selftest --test frag_parity --release \
        > /tmp/g52-selftest.log 2>&1; then
        fail "selftest: the corrupted mnemonic mapping PASSED the gate (G52 blind)"
    else
        msg $GREEN "  G52 selftest: corrupted mapping correctly failed (red path demonstrated)"
    fi
}

# ---------------------------------------------------------------------------
# G53 (P2): the interpreter ↔ hosted-native execution differential. The
# in-scope fixtures run byte-identically through the interpreter and the
# hosted x86_64 binary (`testio.*` surface, I-4/FR-8), trap fixtures pin trap
# code 10 (I-3), and `build_ir`+`render_ir` are byte-identical to
# `langc --emit=ir` (D-1/R-2/DEC-13). The property tests pin the model/alias/
# heap-geometry invariants (R-1/R-3).
# ---------------------------------------------------------------------------

g53_green() {
    msg $YELLOW "G53: interpreter ↔ native execution goldens"

    if cargo test -p wasm-repl --test exec_parity --test props --release \
        > /tmp/g53-parity.log 2>&1; then
        msg $GREEN "  G53: execution differential + property tests green (byte-identical to native; alias probes pinned)"
    else
        fail "execution differential (see /tmp/g53-parity.log)"
    fi
}

g53_selftest() {
    # Red-path demo (AC-9): corrupt `add_i64` → `wrapping_sub` under
    # `wasm-selftest` and REQUIRE the execution differential to fail.
    msg $YELLOW "G53 selftest: corrupted 'add_i64' must red the execution differential"
    if cargo test -p wasm-repl --features wasm-selftest --test exec_parity --release \
        > /tmp/g53-selftest.log 2>&1; then
        fail "selftest: the corrupted AddI64 arm PASSED the gate (G53 blind)"
    else
        msg $GREEN "  G53 selftest: corrupted AddI64 correctly failed (red path demonstrated)"
    fi
}

# ---------------------------------------------------------------------------
# G56 (P3): REPL session goldens byte-exact (DEC-3..DEC-6, §7.2/§7.3). The
# host driver replays every `test-goldens/repl/*.session` and requires the
# produced `tyu.repl-transcript/1` to match the committed `.expected`
# byte-for-byte (persistent stack, transactional rollback, definitions commit
# across phrase faults, deterministic fuel exhaustion, caps diagnostics).
# ---------------------------------------------------------------------------

g56_green() {
    msg $YELLOW "G56: REPL session goldens byte-exact"

    local failed_golden=""
    for f in test-goldens/repl/*.session; do
        if ! cargo run -q -p wasm-repl --features cli --bin repl-cli -- replay "$f" --check \
            > /tmp/g56-replay.log 2>&1; then
            failed_golden="$f"
            break
        fi
    done
    if [ -n "$failed_golden" ]; then
        fail "session golden replay ($failed_golden) — see /tmp/g56-replay.log"
    else
        msg $GREEN "  G56: all test-goldens/repl/*.session transcripts byte-exact"
    fi
}

g56_selftest() {
    # Red-path demo (AC-9): the `wasm-selftest` corruption breaks the DEC-6
    # output-retention asymmetry (a trapped phrase's output is discarded), so
    # the rollback-trap golden MUST mismatch.
    msg $YELLOW "G56 selftest: corrupted rollback must red the session goldens"
    if cargo run -q -p wasm-repl --features wasm-selftest,cli --bin repl-cli \
        -- replay test-goldens/repl/rollback-trap.session --check \
        > /tmp/g56-selftest.log 2>&1; then
        fail "selftest: the corrupted rollback PASSED the session goldens (G56 blind)"
    else
        msg $GREEN "  G56 selftest: corrupted rollback correctly redded the goldens"
    fi
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------

mode="${1:-run}"
case "$mode" in
    run)
        g52_green
        g53_green
        g56_green
        ;;
    --selftest)
        g52_selftest
        g53_selftest
        g56_selftest
        ;;
    *)
        msg $RED "unknown mode '$mode' (run | --selftest)"
        exit 2
        ;;
esac

echo ""
if [ "$failures" -gt 0 ]; then
    msg $RED "ci/wasm.sh ($mode): $failures gate(s) failed — see the FAIL lines above"
    exit 1
fi
msg $GREEN "ci/wasm.sh ($mode): all PLAN-WASM-1 gates defined so far passed."