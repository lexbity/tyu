#!/usr/bin/env bash
# Browser platform CI gates — G52–G58 (declared here, NOT in ci/guards.sh).
#
# The existing `ci/guards.sh` registry ends at G51 (§Q10). The WASM REPL
# plan's new gates live in this file so the flat G1–G51 registry is not
# renumbered/interleaved:
#
#   G52  Fork ↔ `src_interp` fragvec corpus trace agreement            (P1)
#   G53  Interpreter ↔ native execution goldens (+IR render parity)    (P2/P6)
#   G54  codegen-wasm: wasm-tools validate + wasmtime/native parity     (B1/B2)
#   G55  Browser bundle budgets, artifact hashes, now_ms grep, smoke    (P5)
#   G56  REPL session goldens byte-exact                                (P3)
#
# PLAN-WASM-2 (devdocs/plans/web-ide.md) appends to the registry here:
#   G57  Debug-surface parity: native `debug-dump` == core API, and the
#        browser debug panel == the committed native dump                (W4)
#   G58  Examples integrity: total classification, byte-determinism,
#        hash pin, behavioral expectation legs (green→Ok, red→E<code>)   (W3/W5)
# W4 also pins the G57(a) debug-dump parity into the G53 test leg; W6 adds
# the labs-corpus IR-parity test (examples_ir_parity) and the FR-33
# page-fail demonstration to the selftest.
#
# Implemented: G52 (P1), G53 (P2 + W6's labs-corpus IR parity + the G57(a)
# debug-dump parity pin), G56 (P3), G55 (P5 + W2/W4/W6: editor build+budgets,
# debug-console smoke legs, the FR-33 page-fail demonstration), G58 (W3/W5:
# pack, determinism, behavioral legs). G54 is implemented by B1; B2 extends
# its corpus with MMIO and platform-pack acceptance. ci/guards.sh
# carries a pointer comment for the G52-G56 numbering overlap with
# PLAN-RELEASE-1's G52-G55 (DEC-14 note).
#
# Usage:
#   bash ci/wasm.sh            run every gate defined so far
#   bash ci/wasm.sh --selftest run every gate's red-path demo and require
#                              each to FAIL (AC-9 — a gate that cannot be
#                              demonstrated red is not a gate)
#   bash ci/wasm.sh --host-only
#                              run the host-side legs only: G52/G53/G56 in
#                              full, the host legs of G55 (span-table node
#                              tests, now_ms grep), and — under --selftest —
#                              every host-runnable red path. G54 and the
#                              browser-tooled G55 legs need wasm-bindgen/
#                              wasm-tools/wasmtime/playwright and are owned
#                              by the dedicated wasm job in .github/workflows/
#                              ci.yml; a tool-less job must pass --host-only
#                              rather than have the gates silently skip.
#
# Runs standalone on a fresh clone with only the documented toolchain (AC-1).
# `ci/guards.sh` carries a pointer comment for G52–G56 and no gate edits.

set -euo pipefail

RED=1; GREEN=2; YELLOW=3
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
    # `codegen-core` entered the allowlist in P4 for the compiled platform
    # descriptor (`Option<&CompiledDescriptor>` — the target-spec's compiled
    # board identity, §Q5/DEC-8; the plan's R-A "codegen-core (target spec
    # only)" grew exactly by this type). B2 adds codegen-wasm for the
    # artifact-level optional browser backend; that crate has no external
    # dependencies (R-C).
    local bad
    bad=$(
        awk '/^\[dependencies\]/{f=1;next} /^\[/{f=0} f' crates/wasm-repl/Cargo.toml \
            | grep -E '^[a-z][a-z0-9_-]*[[:space:]]*=' \
            | cut -d= -f1 | sed 's/[[:space:]]//g' \
            | grep -vE '^(ir|frontend|semantics|verifier|codegen-core|codegen-wasm)$' || true
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
            # Newer nightlies place the final rlib at release/ top level
            # instead of (or in addition to) a hash-named copy under deps/;
            # the isolated dir makes checking every layout sound.
            for r in target/g52-prod/release/deps/libverifier-*.rlib \
                     target/g52-prod/release/libverifier.rlib; do
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
# G53 (P2/P4): the interpreter ↔ hosted-native execution differential. The
# in-scope fixtures run byte-identically through the interpreter and the
# hosted x86_64 binary (`testio.*` surface, I-4/FR-8), trap fixtures pin trap
# code 10 (I-3), and `build_ir`+`render_ir` are byte-identical to
# `langc --emit=ir` (D-1/R-2/DEC-13). P4 extends the scope to the MMIO
# fixtures: they render byte-equal to `langc --emit=ir` under the platform
# descriptor and run self-check-clean against the scripted device, and the
# mmio unit suite pins the `WriteKind`/field semantics + the E9152/E3640/
# E9156 negatives. The property tests pin the model/alias/heap-geometry
# invariants (R-1/R-3).
# ---------------------------------------------------------------------------

g53_green() {
    msg $YELLOW "G53: interpreter ↔ native execution goldens"

    if cargo test -p wasm-repl --features cli --test exec_parity --test mmio --test props \
        --test debug_dump_parity --test examples_ir_parity --test examples_expectations \
        --release > /tmp/g53-parity.log 2>&1; then
        msg $GREEN "  G53: execution differential + property tests green (byte-identical to native; alias probes pinned; MMIO fixture differential green; G57(a) debug-dump parity pinned; W6 labs IR parity + W5 G58 behavioral expectations pinned)"
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
# G54 (B1): validate every emitted module and compare execution against a
# separately compiled hosted-native oracle over the complete G53 core corpus.
# The test builds both artifacts from the same source fixture, captures the
# hosted binary's testio output/exit result, validates Wasm as MVP, and runs
# the module under wasmtime with the testio host imports. B2 adds MMIO.
# ---------------------------------------------------------------------------

g54_green() {
    msg $YELLOW "G54: codegen-wasm validation and hosted-native parity"
    if cargo build -p langc -p tyu --release > /tmp/g54-build.log 2>&1; then
        if cargo test -p execution-tests --features g54 --test wasm --release \
            > /tmp/g54-parity.log 2>&1; then
            msg $GREEN "  G54: core corpus + MMIO fixtures validate and match hosted-native/register-map expectations"
        else
            fail "G54 wasm validation/native parity (see /tmp/g54-parity.log)"
        fi
    else
        fail "G54 langc/tyu release build (see /tmp/g54-build.log)"
    fi
}

g54_selftest() {
    msg $YELLOW "G54 selftest: a deliberately miscompiled AddI64 must diverge from native"
    if cargo build -p tyu --release > /tmp/g54-selftest-build.log 2>&1 \
        && cargo build -p langc --features wasm-selftest --release \
            >> /tmp/g54-selftest-build.log 2>&1; then
        if cargo test -p execution-tests --features g54 --test wasm \
            exported_main_result_matches_hosted_native_exit_status --release -- --exact \
            > /tmp/g54-selftest.log 2>&1; then
            fail "selftest: deliberately miscompiled AddI64 PASSED native parity (G54 blind)"
        else
            msg $GREEN "  G54 selftest: deliberately miscompiled AddI64 correctly failed native parity"
        fi
    else
        fail "G54 selftest driver build (see /tmp/g54-selftest-build.log)"
    fi
    # Restore the normal compiler artifact: this gate intentionally replaces
    # target/release/langc with a mutation, and later local gates must never
    # accidentally reuse that test binary.
    if cargo build -p langc --release >> /tmp/g54-selftest-build.log 2>&1; then
        msg $GREEN "  G54 selftest: normal langc artifact restored"
    else
        fail "G54 selftest could not restore normal langc (see /tmp/g54-selftest-build.log)"
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

g57_selftest() {
    msg $YELLOW "G57 selftest: corrupted frame ordering must red debug parity"
    if cargo test -p wasm-repl --features wasm-selftest --test debug_abi \
        frames_of_a_suspended_phrase_are_innermost_first --release \
        > /tmp/g57-selftest.log 2>&1; then
        fail "selftest: corrupted FrameCell order PASSED G57 (G57 blind)"
    else
        msg $GREEN "  G57 selftest: corrupted FrameCell order correctly failed parity"
    fi
}

# ---------------------------------------------------------------------------
# G55 (P5): the wasm boundary and the static browser shell. Builds the
# wasm32 cdylib, runs `wasm-bindgen`, packs `tyu-sysroot/1` (AFTER bindgen —
# it cleans its out-dir), generates the SHA-256-pinned artifact manifest the
# page boots against, enforces the NFR-5 byte budgets, proves the build
# reproducible (R-4), greps the engine for `now_ms` (§Q6/G55-a), clippy-gates
# the wasm target, and drives the real browser smoke (both modes).
#
# Budget knobs are env-overridable ONLY for the red-path selftest
# (TYU_G55_WASM_MAX / _JS_ / _SYSROOT_ / _TOTAL_); defaults are NFR-5.
# ---------------------------------------------------------------------------

bytes_of() { stat -c %s "$1"; }

build_web() {
    msg $YELLOW "G55: build-web (wasm32 cdylib, bindgen, sysroot blob, hash pin)"
    cargo build -p wasm-repl --target wasm32-unknown-unknown --release \
        > /tmp/g55-cargo.log 2>&1 || { fail "wasm32 release build (see /tmp/g55-cargo.log)"; return 1; }

    if ! command -v wasm-bindgen >/dev/null 2>&1; then
        fail "wasm-bindgen CLI not installed (cargo install wasm-bindgen-cli --version 0.2.108)"
        return 1
    fi
    wasm-bindgen --target web --out-dir web/pkg \
        target/wasm32-unknown-unknown/release/wasm_repl.wasm \
        > /tmp/g55-bindgen.log 2>&1 || { fail "wasm-bindgen (see /tmp/g55-bindgen.log)"; return 1; }

    # The blob is packed AFTER bindgen (bindgen cleans the out-dir).
    cargo run -q -p wasm-repl --features cli --bin repl-cli -- pack-sysroot web/pkg/tyu-sysroot-1 \
        > /tmp/g55-pack.log 2>&1 || { fail "sysroot blob pack (see /tmp/g55-pack.log)"; return 1; }

    # The example corpus (W3, DEC-18): the book's labs, classified totally —
    # an unclassifiable lab fails the build (the drift detector).
    if [ ! -d docs/book/labs ]; then
        fail "docs/book/labs missing (the example corpus is the packer's input)"
        return 1
    fi
    cargo run -q -p wasm-repl --features cli --bin repl-cli -- pack-examples docs/book/labs web/pkg/examples.js \
        > /tmp/g58-pack.log 2>&1 || { fail "examples pack (see /tmp/g58-pack.log)"; return 1; }
    grep -q "0 unclassified" web/pkg/examples.js || {
        fail "examples pack left unclassified labs (G58)"; return 1
    }

    # The artifact manifest the page boots against (NFR-10 hash pinning).
    local ver wasm_sha sys_sha examples_sha
    ver=$(sed -n 's/.*ENGINE_VERSION: u32 = \([0-9]*\).*/\1/p' crates/wasm-repl/src/lib.rs | head -1)
    wasm_sha=$(sha256sum web/pkg/wasm_repl_bg.wasm | cut -d' ' -f1)
    sys_sha=$(sha256sum web/pkg/tyu-sysroot-1 | cut -d' ' -f1)
    examples_sha=$(sha256sum web/pkg/examples.js | cut -d' ' -f1)
    cat > web/pkg/artifacts.js <<EOF2
// GENERATED by ci/wasm.sh build-web (PLAN-WASM-1 §Q12/G55; W3 adds the
// examples pin) — do not edit. The page refuses to boot/use an artifact
// whose SHA-256 does not match (NFR-10 / I-7).
export const ARTIFACTS = {
  wasm_sha256: "${wasm_sha}",
  sysroot_sha256: "${sys_sha}",
  examples_sha256: "${examples_sha}",
  sysroot_version: "tyu-sysroot/1",
  engine_version: ${ver},
};
EOF2
    # The editor stack (PLAN-WASM-2 W2, DEC-17): the CodeMirror 6 bundle +
    # lang-tyu module, built by esbuild into pkg/vendor/ — and its SRI hash
    # pinned into index.html (the JS-integrity mechanism that works).
    if ! command -v node >/dev/null 2>&1; then
        fail "node not available for the editor build (W2 requires Node >= 20)"
        return 1
    fi
    node web/editor/build.mjs > /tmp/g55-editor.log 2>&1 || {
        fail "editor bundle build (see /tmp/g55-editor.log)"; return 1
    }
    local sri
    sri=$(node -p "JSON.parse(require('fs').readFileSync('web/pkg/vendor/manifest.json','utf8')).editor_sri384")
    # Idempotent injection: replace the shipped placeholder OR a previous
    # build's hash (the build is deterministic, so the committed value is
    # stable; drift means the vendor inputs changed and index.html must be
    # re-committed with the build — never hand-edited).
    if ! grep -q 'pkg/vendor/editor.js' web/index.html; then
        fail "index.html lost its editor script tag (must ship committed)"
        return 1
    fi
    sed -i "s|integrity=\"[^\"]*\" crossorigin=\"anonymous\"></script>|integrity=\"${sri}\" crossorigin=\"anonymous\"></script>|" web/index.html
    if ! grep -q "integrity=\"${sri}\"" web/index.html; then
        fail "editor SRI injection failed (index.html script tag malformed?)"
        return 1
    fi
    msg $GREEN "  G55: build-web complete (engine v${ver}, wasm ${wasm_sha:0:12}…, editor ${sri:0:16}…)"
}

budgets() {
    # Read the knobs HERE (not at source-load time) so the selftest's env
    # overrides actually apply.
    local BUDGET_WASM="${TYU_G55_WASM_MAX:-2097152}"      # wasm raw   <= 2.0 MiB
    # NFR-11 (DEC-17 amendment): the app+glue JS+CSS <= 150 KiB still; the
    # editor vendor bundle adds its own line (<= 400 KiB raw / <= 120 KiB
    # brotli); the combined JS+CSS raw total <= 450 KiB.
    # NFR-11 (W7 amendment): the right-pane tabs, the example filter, and
    # the button/empty-state hierarchy add ~9 KiB of hand-written glue —
    # a feature the 450 KiB cap had no headroom for (it sat 356 bytes under
    # the line). The combined total moves to 480 KiB; the vendor bundle
    # keeps its own caps, so the first-load transfer stays vendor-dominated.
    local BUDGET_JS="${TYU_G55_JS_MAX:-153600}"           # app JS+CSS raw <= 150 KiB
    local BUDGET_EDITOR="${TYU_G55_EDITOR_MAX:-409600}"   # vendor raw <= 400 KiB
    local BUDGET_EDITOR_BROTLI="${TYU_G55_EDITOR_BROTLI_MAX:-122880}" # <= 120 KiB
    local BUDGET_JSTOTAL="${TYU_G55_JS_TOTAL_MAX:-491520}" # JS+CSS total raw <= 480 KiB
    local BUDGET_SYSROOT="${TYU_G55_SYSROOT_MAX:-262144}" # blob       <= 256 KiB
    local BUDGET_TOTAL="${TYU_G55_TOTAL_MAX:-1048576}"    # brotli     <= 1 MiB
    local failed_b=0
    local wasm_bytes js_bytes sys_bytes total=0 f
    wasm_bytes=$(bytes_of web/pkg/wasm_repl_bg.wasm)
    js_bytes=0
    for f in web/app.js web/style.css web/editor_app.mjs web/pkg/wasm_repl.js web/pkg/artifacts.js; do
        js_bytes=$((js_bytes + $(bytes_of "$f")))
    done
    local editor_bytes editor_css_bytes editor_brotli
    editor_bytes=$(bytes_of web/pkg/vendor/editor.js)
    editor_css_bytes=$(bytes_of web/pkg/vendor/editor.css)
    editor_brotli=$(brotli -c -q 11 < web/pkg/vendor/editor.js 2>/dev/null | wc -c || echo 999999999)
    sys_bytes=$(bytes_of web/pkg/tyu-sysroot-1)
    local jstotal=$((js_bytes + editor_bytes + editor_css_bytes))

    [ "$wasm_bytes" -le "$BUDGET_WASM" ] || { fail "wasm budget: $wasm_bytes > $BUDGET_WASM (NFR-5)"; failed_b=1; }
    [ "$js_bytes" -le "$BUDGET_JS" ] || { fail "JS+CSS budget: $js_bytes > $BUDGET_JS (NFR-5)"; failed_b=1; }
    [ "$editor_bytes" -le "$BUDGET_EDITOR" ] || { fail "editor bundle budget: $editor_bytes > $BUDGET_EDITOR (NFR-11)"; failed_b=1; }
    [ "$editor_brotli" -le "$BUDGET_EDITOR_BROTLI" ] || { fail "editor brotli budget: $editor_brotli > $BUDGET_EDITOR_BROTLI (NFR-11)"; failed_b=1; }
    [ "$jstotal" -le "$BUDGET_JSTOTAL" ] || { fail "JS+CSS total budget: $jstotal > $BUDGET_JSTOTAL (NFR-11)"; failed_b=1; }
    [ "$sys_bytes" -le "$BUDGET_SYSROOT" ] || { fail "sysroot blob budget: $sys_bytes > $BUDGET_SYSROOT (NFR-5)"; failed_b=1; }
    # SRI presence (FR-22): the served page pins the editor script.
    if [ "$(grep -c 'integrity="sha384-' web/index.html)" -lt 1 ]; then
        fail "index.html carries no SRI-pinned editor script (FR-22)"
        failed_b=1
    fi

    # First-load transfer, brotli-compressed, summed over every fetched asset.
    if command -v brotli >/dev/null 2>&1; then
        for f in web/pkg/wasm_repl_bg.wasm web/pkg/wasm_repl.js web/pkg/artifacts.js web/app.js web/style.css web/index.html web/pkg/tyu-sysroot-1 web/pkg/vendor/editor.js web/pkg/vendor/editor.css; do
            total=$((total + $(brotli -c -q 11 < "$f" | wc -c)))
        done
        [ "$total" -le "$BUDGET_TOTAL" ] || { fail "first-load brotli budget: $total > $BUDGET_TOTAL (NFR-5)"; failed_b=1; }
    else
        msg $YELLOW "  G55: (brotli unavailable; compressed total skipped)"
    fi

    if [ "$failed_b" -eq 0 ]; then
        msg $GREEN "  G55: budgets hold (wasm ${wasm_bytes}B, JS+CSS ${js_bytes}B, blob ${sys_bytes}B, brotli total ${total}B)"
    fi
    return $failed_b
}

reproducibility() {
    # R-4: the artifact hash must be build-deterministic. Relink the crate
    # (deps stay cached) and compare the fresh artifact byte-for-byte.
    local before after
    before=$(sha256sum web/pkg/wasm_repl_bg.wasm | cut -d' ' -f1)
    touch crates/wasm-repl/src/lib.rs
    cargo build -p wasm-repl --target wasm32-unknown-unknown --release > /dev/null 2>&1
    wasm-bindgen --target web --out-dir /tmp/g55-repro \
        target/wasm32-unknown-unknown/release/wasm_repl.wasm > /dev/null 2>&1
    after=$(sha256sum /tmp/g55-repro/wasm_repl_bg.wasm | cut -d' ' -f1)
    rm -rf /tmp/g55-repro
    if [ "$before" = "$after" ]; then
        msg $GREEN "  G55: wasm artifact build is reproducible (${before:0:12}…)"
    else
        fail "wasm artifact NOT reproducible across rebuilds ($before vs $after) — R-4"
    fi
    # NFR-14: the editor bundle is byte-reproducible too (fixed inputs, no
    # timestamps — two consecutive builds hash-identical).
    local ebefore eafter
    ebefore=$(sha256sum web/pkg/vendor/editor.js | cut -d' ' -f1)
    node web/editor/build.mjs > /dev/null 2>&1
    eafter=$(sha256sum web/pkg/vendor/editor.js | cut -d' ' -f1)
    if [ "$ebefore" = "$eafter" ]; then
        msg $GREEN "  G55: editor bundle build is reproducible (${ebefore:0:12}…)"
    else
        fail "editor bundle NOT reproducible across builds ($ebefore vs $eafter) — NFR-14"
    fi
    # G58: the examples pack is byte-reproducible (sorted emit, fixed escapes).
    local xbefore xafter
    xbefore=$(sha256sum web/pkg/examples.js | cut -d' ' -f1)
    cargo run -q -p wasm-repl --features cli --bin repl-cli -- pack-examples docs/book/labs /tmp/g58-repro.js \
        > /dev/null 2>&1 || { fail "examples re-pack (determinism leg)"; return 1; }
    xafter=$(sha256sum /tmp/g58-repro.js | cut -d' ' -f1)
    rm -f /tmp/g58-repro.js
    if [ "$xbefore" = "$xafter" ]; then
        msg $GREEN "  G58: examples pack is byte-reproducible (${xbefore:0:12}…)"
    else
        fail "examples pack NOT byte-identical across runs ($xbefore vs $xafter) — G58"
    fi
}

now_ms_grep() {
    # §Q6/G55-a: the engine MUST NOT call `now_ms`. The abi module's import
    # declaration is the single sanctioned mention.
    if grep -rln "now_ms" crates/wasm-repl/src --include='*.rs' | grep -v 'src/abi.rs' > /dev/null; then
        fail "now_ms referenced outside the abi module (engine determinism, §Q6)"
    else
        msg $GREEN "  G55: now_ms absent from the engine modules (§Q6/G55-a)"
    fi
}

wasm_clippy() {
    if cargo clippy -p wasm-repl --target wasm32-unknown-unknown --release -- -D warnings \
        > /tmp/g55-clippy.log 2>&1; then
        msg $GREEN "  G55: clippy clean on the wasm32 target (-D warnings)"
    else
        fail "wasm32 clippy (see /tmp/g55-clippy.log)"
    fi
}

smoke() {
    local mode="$1"
    local log="/tmp/g55-smoke${mode:+-tampered}.log"
    if ! node --version > /dev/null 2>&1; then
        fail "node not available for the browser smoke"
        return
    fi
    if ! (cd web/test && node -e "import('playwright').catch(() => process.exit(1))") > /dev/null 2>&1; then
        if [ -n "${CI:-}" ]; then
            fail "playwright not installed under CI (web/test: npm install playwright)"
        else
            msg $YELLOW "  G55: (playwright unavailable; browser smoke skipped locally)"
        fi
        return
    fi
    if (cd web/test && node smoke.mjs "$mode" > "$log" 2>&1); then
        msg $GREEN "  G55: browser smoke${mode:+ (tampered red path)} green"
    else
        fail "browser smoke${mode:+ (tampered)} (see $log)"
    fi
}

g55_green() {
    msg $YELLOW "G55: the wasm boundary and the static browser shell"
    wasm_clippy
    if node --test web/test/*.test.mjs > /tmp/g55-node-tests.log 2>&1; then
        msg $GREEN "  G55: browser span-map and Wasm span-table tests green"
    else
        fail "browser span-map/Wasm span-table tests (see /tmp/g55-node-tests.log)"
    fi
    if build_web; then
        budgets
        reproducibility
        now_ms_grep
        smoke ""
    fi
}

g55_host_only() {
    # The host-runnable legs of G55 for tool-less jobs (--host-only): the
    # span-table unit tests are plain node (no npm deps) and the now_ms grep
    # is plain grep. The wasm32 clippy, the bindgen bundle build, the byte
    # budgets, the reproducibility re-link, and the browser smoke need the
    # wasm toolchain/playwright provisioned by the dedicated wasm job —
    # skipping them here is a provisioning decision recorded in the workflow,
    # never an automatic fallback (a missing tool in the wasm job must red,
    # not skip).
    msg $YELLOW "G55: host-side legs (--host-only; browser-tooled legs are owned by the wasm job)"
    if node --test web/test/*.test.mjs > /tmp/g55-node-tests.log 2>&1; then
        msg $GREEN "  G55: browser span-map and Wasm span-table tests green"
    else
        fail "browser span-map/Wasm span-table tests (see /tmp/g55-node-tests.log)"
    fi
    now_ms_grep
}

g58_selftest() {
    # Red-path demo (W3/G58): the drift detector — a lab outside the
    # classification convention must FAIL the pack (never a silent drop).
    # Host-only (repl-cli pack-examples), so it runs in every mode.
    msg $YELLOW "G58 selftest: an unclassifiable lab must red the pack"
    rm -rf /tmp/g58-red-labs
    cp -r docs/book/labs /tmp/g58-red-labs
    echo 'module Junk;' > /tmp/g58-red-labs/ch06/zz-mystery-lab.mod
    if cargo run -q -p wasm-repl --features cli --bin repl-cli -- pack-examples /tmp/g58-red-labs /tmp/g58-red.js         > /tmp/g58-red.log 2>&1; then
        fail "selftest: the pack PASSED with an unclassifiable lab (G58 blind)"
    else
        msg $GREEN "  G58 selftest: the unclassifiable lab correctly redded the pack"
    fi
    rm -rf /tmp/g58-red-labs /tmp/g58-red.js
}

g55_selftest() {
    # Red-path demo 1 (AC-9): a tampered artifact must fail the page's
    # integrity check LOUDLY — the tampered smoke run is itself green (it
    # asserts the failure banner appears and no engine boots).
    msg $YELLOW "G55 selftest: tampered wasm artifact must red the page's integrity check"
    smoke "--tampered"

    # Red-path demo (W6/FR-33): a post-boot wasm exception must route to
    # the full-page failure view — the un-guarded submit path is closed.
    msg $YELLOW "G55 selftest: a post-boot wasm exception must page-fail loudly"
    if (cd web/test && node smoke.mjs --pagefail > /tmp/g55-pagefail.log 2>&1); then
        msg $GREEN "  G55 selftest: the page-fail path correctly demonstrated"
    else
        fail "page-fail demonstration (see /tmp/g55-pagefail.log)"
    fi

    g58_selftest

    # Red-path demo 2: with a budget knob forced absurdly low, the budget
    # check MUST fail (proving the check is consulted, not decorative). The
    # expected failure lines are rolled back out of the run's failure count.
    msg $YELLOW "G55 selftest: an absurdly low budget must red the gate"
    local saved_failures=$failures
    if TYU_G55_WASM_MAX=1 TYU_G55_JS_MAX=1 TYU_G55_EDITOR_MAX=1 TYU_G55_EDITOR_BROTLI_MAX=1 TYU_G55_JS_TOTAL_MAX=1 TYU_G55_SYSROOT_MAX=1 TYU_G55_TOTAL_MAX=1 budgets > /tmp/g55-budget-red.log 2>&1; then
        fail "selftest: the budget check PASSED with 1-byte budgets (G55 blind)"
    else
        msg $GREEN "  G55 selftest: 1-byte budgets correctly redded the check"
        failures=$saved_failures
    fi
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------

host_only=0
mode="run"
for arg in "$@"; do
    case "$arg" in
        run)         mode="run" ;;
        --selftest)  mode="--selftest" ;;
        --host-only) host_only=1 ;;
        *)
            msg $RED "unknown argument '$arg' (run | --selftest | --host-only)"
            exit 2
            ;;
    esac
done

case "$mode" in
    run)
        g52_green
        g53_green
        if [ "$host_only" -eq 1 ]; then
            msg $YELLOW "G54: skipped (--host-only: wasm-tools/wasmtime validation legs are owned by the wasm job)"
        else
            g54_green
        fi
        g56_green
        if [ "$host_only" -eq 1 ]; then
            g55_host_only
        else
            g55_green
        fi
        ;;
    --selftest)
        g52_selftest
        g53_selftest
        if [ "$host_only" -eq 1 ]; then
            msg $YELLOW "G54 selftest: skipped (--host-only: owned by the wasm job)"
        else
            g54_selftest
        fi
        g56_selftest
        g57_selftest
        if [ "$host_only" -eq 1 ]; then
            g58_selftest
        else
            g55_selftest
        fi
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
msg $GREEN "ci/wasm.sh ($mode): all browser platform gates passed."
