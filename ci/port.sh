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
# forbidden constant fails the gate. The audited set is OWNED by
# `REVIEW.md` §3: every theorem named there MUST appear in the audit
# output — an audit that silently drops a registry theorem fails here
# (fail-closed against audit erosion).
if [ -f AxiomAudit.lean ] && [ -f REVIEW.md ]; then
    msg 2 "  port.sh: axiom audit (AxiomAudit.lean, registry owned by REVIEW.md §3)"
    AUDIT_OUT="$(lake env lean AxiomAudit.lean 2>&1 || true)"
    # every printed axiom set must belong to the permitted set
    if printf '%s\n' "$AUDIT_OUT" | grep -qE "sorryAx|Lean.ofReduceBool"; then
        msg 1 "  port.sh: axiom audit FAIL — a registry theorem depends on a forbidden axiom"
        printf '%s\n' "$AUDIT_OUT" | grep -E "depends on axioms" >&2
        exit 1
    fi
    audited="$(printf '%s\n' "$AUDIT_OUT" | grep -cE "depends on axioms|does not depend on any axioms" || true)"
    # registry coverage: each REVIEW.md §3 theorem name must be audited
    # (both green forms: a permitted-axiom set, or an empty axiom set)
    missing=0
    while IFS= read -r thm; do
        if ! printf '%s\n' "$AUDIT_OUT" | grep -qE "^'$thm' (depends on axioms|does not depend on any axioms)"; then
            msg 1 "  port.sh: axiom audit FAIL — registry theorem not audited: $thm"
            missing=1
        fi
    done < <(sed -n '/^## 3\./,/^## /p' REVIEW.md | sed -n '/^```text$/,/^```$/p' | grep "^Tyu\." || true)
    if [ "$missing" -ne 0 ]; then
        msg 1 "  port.sh: axiom audit FAIL — REVIEW.md §3 ↔ AxiomAudit.lean out of sync"
        exit 1
    fi
    if [ "$audited" -lt 10 ]; then
        msg 1 "  port.sh: axiom audit FAIL — only $audited theorems audited (expected ≥ 10)"
        exit 1
    fi
    msg 2 "  port.sh: axiom audit green ($audited registry theorems, permitted set only, REVIEW.md coverage exact)"
else
    msg 1 "  port.sh: AxiomAudit.lean / REVIEW.md missing (P4.2/P4.3 gate)"
    exit 1
fi

# --- PLAN-VERIFY-3 P4.2 — stackmeta replay (T-C's empirical hook) ---
# Every corpus word's declared (net, high) is re-derived by the port's own
# walk + monoid: net must match exactly and the peak envelope must stay
# within the declared bound; a corrupted declared value exits nonzero.
# An unresolved call SKIPS a word — a silent skip would quietly shrink the
# theorem's empirical coverage, so any skip > 0 fails the gate (fail-closed
# against coverage erosion; a legitimate skip is a reviewed corpus change).
STACKMETA_GOLDENS=("$ROOT"/test-goldens/stackmeta/*/*.json)
msg 2 "  port.sh: stackmeta replay (${#STACKMETA_GOLDENS[@]} golden files)"
STATUS=0
STACKMETA_OUT="$(.lake/build/bin/conformance --level stackmeta --stackmeta "${STACKMETA_GOLDENS[@]}")" || STATUS=$?
printf '%s\n' "$STACKMETA_OUT"
if [ "$STATUS" -ne 0 ]; then
    msg 1 "  port.sh: stackmeta replay FAILED (divergence or unreadable golden)"
    exit 1
fi
skipped="$(printf '%s\n' "$STACKMETA_OUT" | sed -n 's/.*skipped=\([0-9]*\).*/\1/p' | tail -1)"
if [ "${skipped:-0}" -ne 0 ]; then
    msg 1 "  port.sh: stackmeta replay FAILED — $skipped word(s) skipped (unresolved calls); zero skips required"
    exit 1
fi
msg 2 "  port.sh: stackmeta replay green (0 divergences, 0 skipped)"
# --- PLAN-VERIFY-3 P9.3 — the fragment-vector conformance corpus ---
# The shared `tyu.fragvec/1` corpus (program → trace) is the fragment
# surface's mechanical Lean↔Rust pin: the port parses the SAME canonical
# `--emit=ir` block text the Rust generator emits
# (`crates/verifier/tests/fragment_vectors.rs`, `blocks_to_text`) and must
# reproduce the committed observable traces byte-for-byte
# (`Tyu/Conformance/Fragment.lean` + `src_interp.blocks_from_text` on the
# Rust side). A fragment-op drift on EITHER side — a misparsed mnemonic or a
# wrong step (the `swap` bug this corpus caught) — diverges here.
FRAG_CORPUS=("$ROOT/crates/verifier/test-vectors/fragment")
msg 2 "  port.sh: P9.3 fragment-vector conformance corpus ($(/usr/bin/env wc -l < "$FRAG_CORPUS/index.json" | tr -d ' ') lines)"
FRAG_OUT="$(.lake/build/bin/conformance --level fragment --corpus "${FRAG_CORPUS[@]}")" || {
    printf '%s\n' "$FRAG_OUT"
    msg 1 "  port.sh: P9.3 fragment corpus DIVERGED (Lean↔Rust fragment drift)"
    exit 1
}
printf '%s\n' "$FRAG_OUT"
echo "$FRAG_OUT" | grep -q "RESULT: fragment=.*mismatches=0" || {
    msg 1 "  port.sh: P9.3 fragment corpus failed (nonzero mismatches)"
    exit 1
}
msg 2 "  port.sh: P9.3 fragment corpus green (zero divergence, Lean↔Rust pin OK)"
# --- PLAN-VERIFY-3 P5 — the statement renderer + Gen goldens (the drift lock) ---
# The `gen` renderer (a pure function of the `tyu.obl/v2` artifacts → the
# generated statements) must (1) pass its SHA-256 self-check, (2) regenerate
# the committed golden statements byte-for-byte (the committed goldens are
# imported by `Tyu.Gen.Golden` in the package — the elaboration gate), and
# (3) agree with the Rust encoder on every statement hash, checked on the
# Rust side by `crates/tooling-tests/tests/gen_render_drift.rs`. Any drift
# fails here (fail-closed).
msg 2 "  port.sh: P5 gen self-check (SHA-256 known-answer vectors)"
if ! .lake/build/bin/gen --selfcheck | grep -q PASS; then
    msg 1 "  port.sh: gen --selfcheck FAILED"
    exit 1
fi
GEN_TMP="$(mktemp -d)"
OBL_SRC=("$ROOT"/verification/ports/lean/goldens/obl/*.obl.json)
msg 2 "  port.sh: P5 gen --render over ${#OBL_SRC[@]} corpus artifacts"
if .lake/build/bin/gen --render --obl "${OBL_SRC[@]}" --out "$GEN_TMP" | grep -q "FAIL"; then
    msg 1 "  port.sh: gen --render reported a FAIL"
    exit 1
fi
# compare the .lean statements (the rendered statements must match the
# committed goldens byte-for-byte)
GENLEAN_DIR="$(mktemp -d)"
mkdir -p "$GENLEAN_DIR"
for f in "$GEN_TMP"/*.lean; do
    [ -f "$f" ] && cp "$f" "$GENLEAN_DIR/"
done
if ! diff -r "$GENLEAN_DIR" "$ROOT/verification/ports/lean/Tyu/Gen/Golden" >/dev/null 2>&1; then
    msg 1 "  port.sh: gen output drifted from the committed golden statements (Tyu/Gen/Golden)"
    diff -r "$GENLEAN_DIR" "$ROOT/verification/ports/lean/Tyu/Gen/Golden" | head -10
    exit 1
fi
rm -rf "$GENLEAN_DIR"
# the metadata (gen.json) files compare against goldens/gen; the .lean
# statements already compared against Tyu/Gen/Golden above
GENJSON_DIR="$(mktemp -d)"
mkdir -p "$GENJSON_DIR"
for f in "$GEN_TMP"/*.gen.json; do
    [ -f "$f" ] && cp "$f" "$GENJSON_DIR/"
done
if ! diff -r "$GENJSON_DIR" "$ROOT/verification/ports/lean/goldens/gen" >/dev/null 2>&1; then
    msg 1 "  port.sh: P5 golden metadata drifted (goldens/gen)"
    diff -r "$GENJSON_DIR" "$ROOT/verification/ports/lean/goldens/gen" | head -10
    exit 1
fi
rm -rf "$GENJSON_DIR"
msg 2 "  port.sh: P5 golden statements + metadata byte-stable (drift lock)"
rm -rf "$GEN_TMP"
msg 2 "  port.sh: P5 elaboration gate — golden statements build (Tyu.Gen.Golden)"
lake build Tyu.Gen.Golden || {
    msg 1 "  port.sh: golden statements failed to elaborate"
    exit 1
}

echo ""
# --- PLAN-VERIFY-3 P6 — the developer-proof pipeline end to end (tier A) ---
# With the pinned toolchain in hand, drive the REAL `tyu build
# --verify-tool=lean` path: package generation into `.tyu-verify/lean/`,
# the E6418 Gen-digest gate, the elaborating lake build, and the honest
# per-module unproven accounting (harvest is a P7 deliverable — nothing is
# claimed proven). This runs the `build_verify_integration` tier-A test
# (env-gated so toolchain-less CI tiers skip it; the port gate ALWAYS has
# the toolchain, so here it runs unconditionally).
msg 2 "  port.sh: P6 developer-proof pipeline e2e (build_verify_integration tier A)"
(
    cd "$ROOT"
    if [ ! -f target/debug/tyu ]; then
        cargo build -q -p tyu -p langc
    fi
    TYU_PROOF_E2E=1 cargo test -q -p tyu --test build_verify_integration 2>&1 | tail -4
) || {
    msg 1 "  port.sh: P6 developer-proof pipeline e2e FAILED"
    exit 1
}
msg 2 "  port.sh: P6 developer-proof pipeline e2e green"

# --- PLAN-VERIFY-3 P7.1 — the harvest gate ---
# The harvest fixture demonstrates the kernel-checked path end to end:
# statement binding (E6420 when a statement's def is missing), the type-level
# theorem binding, the axiom audit (only the benign set; sorryAx ⇒ E6419),
# and byte-deterministic `tyu.verdicts/v2` emission. The clean variant must
# produce the committed golden byte-for-byte; a `sorry` variant MUST fail
# with a nonzero exit and an E6419 error document.
msg 2 "  port.sh: P7.1 harvest gate (harvest fixture + determinant v2 golden)"
HV="$ROOT/verification/ports/lean/tests/harvest-fixture"
HARVEST_GOOD=1
(
    cd "$HV"
    cp "$ROOT/verification/ports/lean/lean-toolchain" lean-toolchain
    lake build TinyFix Tyu.Verdicts.Harvest >/dev/null 2>&1 || exit 1
    # clean variant (both theorems by `trivial`) → golden, byte-for-byte.
    TYU_HARVEST_GEN_DIR="$HV" TYU_HARVEST_OBL="$HV/Tiny.obl.json" \
      TYU_HARVEST_OUT="$HV/tiny-clean.v2.json" \
      lake env lean "$HV/hvharvest.lean" >/dev/null 2>&1 || exit 1
    cmp "$HV/tiny-clean.v2.json" "$ROOT/test-goldens/harvest/Tiny.verdicts.v2.json" >/dev/null 2>&1 || exit 1
    # the axiom-audit evidence travels with the verdicts (tyu.axiom-audit/1).
    grep -q '"schema":"tyu.axiom-audit/1"' "$HV/tiny-clean.v2.json.audit.json" || exit 1
    # sorry variant: rewrite theorem 1's proof to `sorry`, rebuild, and expect
    # a nonzero harvest exit (E6419) with the error document present.
    python3 - "$HV/TinyFix.lean" "$HV/TinyFix.sorry.lean" <<'PYEOF'
import sys
src, dst = sys.argv[1], sys.argv[2]
text = open(src).read()
old = """theorem obl_Tiny_inc_subtype_range_1 : stmt_Tiny_inc_subtype_range_1 := by
  trivial"""
new = """theorem obl_Tiny_inc_subtype_range_1 : stmt_Tiny_inc_subtype_range_1 := by
  sorry"""
assert old in text, "fixture shape changed — harvest gate must be reviewed"
open(dst, "w").write(text.replace(old, new))
PYEOF
    mv TinyFix.lean TinyFix.lean.clean
    cp TinyFix.sorry.lean TinyFix.lean
    lake build TinyFix >/dev/null 2>&1 || { mv TinyFix.lean.clean TinyFix.lean; exit 1; }
    if TYU_HARVEST_GEN_DIR="$HV" TYU_HARVEST_OBL="$HV/Tiny.obl.json" \
         TYU_HARVEST_OUT="$HV/tiny-sorry.v2.json" \
         lake env lean "$HV/hvharvest.lean" >/dev/null 2>&1; then
        exit 1  # a sorry theorem must NOT harvest cleanly
    fi
    grep -q "sorryAx" "$HV/tiny-sorry.v2.json" || exit 1
    mv TinyFix.lean.clean TinyFix.lean
    rm -f TinyFix.sorry.lean tiny-clean.v2.json tiny-clean.v2.json.audit.json tiny-sorry.v2.json tiny-sorry.v2.json.audit.json
    # delete-theorem variant: every statement harvests `open` (a state, not a
    # fault) — never a crash, never a fabricated proof.
    "$ROOT/verification/ports/lean/tests/tamper/run-delete-theorem.sh" "$HV" >/dev/null 2>&1 || exit 1
    # mutate-gen variant: a tampered Gen def name fails closed with E6420.
    "$ROOT/verification/ports/lean/tests/tamper/run-mutate-gen.sh" "$HV" >/dev/null 2>&1 || exit 1
    # Keep the fixture pristine: the copied pin and lake artifacts are
    # build-time only (regenerated by the gate each run).
    rm -f lean-toolchain lake-manifest.json
) || {
    ( cd "$HV"; [ -f TinyFix.lean.clean ] && mv TinyFix.lean.clean TinyFix.lean || true; rm -f TinyFix.sorry.lean tiny-sorry.v2.json tiny-sorry.v2.json.audit.json tiny-clean.v2.json tiny-clean.v2.json.audit.json )
    msg 1 "  port.sh: P7.1 harvest gate FAILED"
    exit 1
}
msg 2 "  port.sh: P7.1 harvest gate green (certificate + E6419 + axiom-audit evidence + delete-theorem-open + mutate-gen-E6420 + deterministic golden)"

# --- PLAN-VERIFY-3 P9 — the source-surface worked example (T-S) ---
# The `Sum` fixture's obligation is PROVEN at the SOURCE surface — a
# theorem of the generated `src_stmt_Sum_answer_subtype_range_0` over
# `Tyu/Src.lean` — and the harvest must certify it with `surface: "source"`
# and `proof.relies: ["T-S"]` (§Q2/§Q6/§6.3). This exercises the whole P9
# slice end to end: the renderer's source statement forms, the
# `Tyu.Sound.transcription` T-S registry theorem the reliance names, and the
# harvest's source-surface binding. The repo-level test
# `crates/tooling-tests/tests/source_surface_e2e.rs` drives the identical
# flow; the gate here runs it through the port shell.
msg 2 "  port.sh: P9.1/P9.2 source-surface certificate gate (Sum fixture)"
SF="$ROOT/verification/ports/lean/tests/source-fixture"
SF_TMP="$(mktemp -d)"
(
    cp "$SF/Sum.obl.json" "$SF_TMP/"
    cp "$ROOT/verification/ports/lean/lean-toolchain" "$SF_TMP/lean-toolchain"
    # 1. the port's gen renderer produces the source-surface statement.
    .lake/build/bin/gen --render --obl "$SF_TMP/Sum.obl.json" --out "$SF_TMP/Gen" >/dev/null || exit 1
    grep -q "def src_stmt_Sum_answer_subtype_range_0 : Prop" "$SF_TMP/Gen/Sum.lean" || exit 1
    grep -q "Tyu.Src.outInRange" "$SF_TMP/Gen/Sum.lean" || exit 1
    grep -q '"src_def": "src_stmt_Sum_answer_subtype_range_0"' "$SF_TMP/Gen/Sum.gen.json" || exit 1
    # 2. assemble the package around the fixture proof + generated Gen.
    cp "$SF/SumFix.lean" "$SF_TMP/"
    cp "$SF/hvharvest.lean" "$SF_TMP/"
    cat > "$SF_TMP/lakefile.toml" <<LAKEEOF
name = "tyu-source-suite"
version = "0.1.0"

[[lean_lib]]
name = "Tyu"
srcDir = "$ROOT/verification/ports/lean"
roots = ["Tyu"]

[[lean_lib]]
name = "Gen"
srcDir = "."
roots = ["Gen"]

[[lean_lib]]
name = "SumFix"
srcDir = "."
roots = ["SumFix"]
LAKEEOF
    # 3. build + harvest; the verdict MUST carry the source provenance.
    ( cd "$SF_TMP" && lake build SumFix Tyu.Verdicts.Harvest >/dev/null 2>&1 ) || exit 1
    ( cd "$SF_TMP" && TYU_HARVEST_GEN_DIR="$SF_TMP/Gen" TYU_HARVEST_OBL="$SF_TMP/Sum.obl.json" \
      TYU_HARVEST_OUT="$SF_TMP/out.v2.json" \
      lake env lean "$SF_TMP/hvharvest.lean" >/dev/null 2>&1 ) || exit 1
    grep -q '"surface":"source"' "$SF_TMP/out.v2.json" || exit 1
    grep -Fq '"relies":["T-S"]' "$SF_TMP/out.v2.json" || exit 1
    grep -q '"trust":"proof"' "$SF_TMP/out.v2.json" || exit 1
    grep -q '"status":"ok"' "$SF_TMP/out.v2.json.audit.json" || exit 1
    rm -rf "$SF_TMP"
) || {
    rm -rf "$SF_TMP"
    msg 1 "  port.sh: P9 source-surface gate FAILED (renderer source forms / T-S reliance / harvest binding)"
    exit 1
}
msg 2 "  port.sh: P9 source-surface gate green (src_stmt rendering + surface:source + relies [T-S] + axiom audit)"

echo ""
# --- PLAN-VERIFY-3 P10 — automation: rate measurement + candidate fill ---
# The automation library (`Tyu.Automation`) ships in the port; the
# `automation_rate` executable MEASURES the corpus auto-discharge rate per
# obligation kind (every `closed` row is kernel-checked — the rate is
# measured, never assumed), and the `fill` executable generates the
# marker-headed candidate files (`-- tyu:candidate obligation=<id>`) the
# harvest attributes (`authored: "candidate"`, P10.2). The measured JSON is
# compared against the committed baseline (`ci/automation-rate.json`): a
# regression in the per-kind rate (>10pts) or a closed→open flop on any row
# fails the gate (the NFR-6 process bar, §Q10: published, drifting
# auto-discharge is a reviewed change).
msg 2 "  port.sh: P10 automation gate (automation_rate + fill, baseline-compared)"
AUTOMATION_TMP="$(mktemp -d)"
(
    lake build automation_rate fill >/dev/null 2>&1 || exit 1
    .lake/build/bin/automation_rate --selfcheck || exit 1
    # rate over the corpus goldens, written beside the committed baseline
    AUTOMATION_OBL=()
    AUTOMATION_META=()
    for f in "$ROOT"/verification/ports/lean/goldens/obl/*.obl.json; do
        AUTOMATION_OBL+=( "--obl=$f" )
        AUTOMATION_META+=( "--meta=$ROOT/verification/ports/lean/goldens/gen/$(basename "$f" .obl.json).gen.json" )
    done
    if ! .lake/build/bin/automation_rate "${AUTOMATION_OBL[@]}" "${AUTOMATION_META[@]}" \
        --out="$AUTOMATION_TMP/rate.json" --budget=20 > "$AUTOMATION_TMP/rate.log" 2>&1; then
        msg 1 "  port.sh: automation_rate failed"
        cat "$AUTOMATION_TMP/rate.log" >&2
        exit 1
    fi
    # The functional P10 contract first (always exercised, blocking): fill
    # produces the marker-headed candidates; the marker + the theorem shape
    # are sanity-checked (the harvest's attribution contract).
    .lake/build/bin/fill --selfcheck || exit 1
    .lake/build/bin/fill "${AUTOMATION_OBL[@]}" "${AUTOMATION_META[@]}" --out="$AUTOMATION_TMP/cand" >/dev/null 2>&1 || exit 1
    if ! grep -rq -- "-- tyu:candidate obligation=" "$AUTOMATION_TMP/cand"; then
        msg 1 "  port.sh: fill produced no marker-headed candidates"
        exit 1
    fi
    if [ -f "$ROOT/ci/automation-rate.json" ]; then
        BASELINE_RATE="$(python3 -c "import json;d=json.load(open('$ROOT/ci/automation-rate.json'));print(json.dumps({k['kind']: (k['closed'], k['obligations']) for k in d['by_kind']}))")"
        NEW_RATE="$(python3 -c "import json;d=json.load(open('$AUTOMATION_TMP/rate.json'));print(json.dumps({k['kind']: (k['closed'], k['obligations']) for k in d['by_kind']}))")"
        if [ "$BASELINE_RATE" != "$NEW_RATE" ]; then
            msg 1 "  port.sh: automation rate drifted from the committed baseline"
            msg 1 "           baseline: $BASELINE_RATE"
            msg 1 "           measured: $NEW_RATE"
            msg 1 "           update ci/automation-rate.json (a reviewed change), or fix the automation"
            exit 1
        fi
    else
        msg 3 "  port.sh: no ci/automation-rate.json baseline — committing one"
        cp "$AUTOMATION_TMP/rate.json" "$ROOT/ci/automation-rate.json"
    fi
    msg 2 "  port.sh: P10 candidate e2e (candidate_e2e tier A)"
    ( cd "$ROOT" && TYU_CANDIDATE_E2E=1 cargo test -q -p tooling-tests --test candidate_e2e 2>&1 | tail -3 )
    # The standalone `tyu proof fill <input.mod>` form (P10.2 extraction path).
    ( cd "$ROOT" && TYU_PROOF_E2E=1 cargo test -q -p tyu --test proof_fill_e2e 2>&1 | tail -3 )
    # P10.2 NFR-6 bar (the plan's python gate, vacuity-corrected): every
    # LOOP-FREE kind with obligations is reported against the ≥ 90% bar; a
    # kind with zero loop-free obligations passes vacuously (rate = 100).
    #
    # Posture (§Q10, normative): automation quality is "measured, published,
    # not gated — the bar is economic honesty, not a pass/fail gate", and the
    # P10.2 process gate is "informational, not a build gate". The ABSOLUTE
    # bar therefore REPORTS here (loudly, with the honest per-kind numbers)
    # but does not block the port gate while the recorded divergence is open;
    # the REGRESSION bar above (no drift vs the committed baseline) is the
    # blocking part. Meeting the absolute bar is the concrete-run discharge
    # engine workstream; the divergence is recorded in
    # verification/ports/lean/README.md §P10.
    msg 2 "  port.sh: P10.2 NFR-6 bar (loop-free auto-discharge ≥ 0.9, reported)"
    RATE_BAR_FAIL=0
    python3 - "$AUTOMATION_TMP/rate.json" <<'PYEOF' || RATE_BAR_FAIL=1
import json, sys
d = json.load(open(sys.argv[1]))
for row in d["by_loopfree_kind"]:
    n = int(row["loopfree_obligations"])
    c = int(row["closed"])
    rate = 100 if n == 0 else (c * 100 // n)
    ok = rate >= 90
    print(f"  loopfree {row['kind']}: {c}/{n} closed (rate {rate}%)", "PASS" if ok else "FAIL")
    if not ok:
        sys.exit(1)
PYEOF
    if [ "$RATE_BAR_FAIL" -ne 0 ]; then
        msg 1 "  port.sh: P10.2 NFR-6 bar NOT met (RECORDED DIVERGENCE, non-blocking per §Q10)"
        msg 1 "           the honest per-kind numbers above are the published rate"
        msg 1 "           tracked workstream: the concrete-run discharge engine (Tyu.Automation)"
        msg 1 "           recorded divergence: verification/ports/lean/README.md §P10"
    fi
) || {
    rm -rf "$AUTOMATION_TMP"
    msg 1 "  port.sh: P10 automation gate FAILED"
    exit 1
}
cp "$AUTOMATION_TMP/rate.json" "$ROOT/ci/automation-rate.json"
rm -rf "$AUTOMATION_TMP"
msg 2 "  port.sh: P10 automation gate green (rate measured, baseline locked, fill markers + candidate attribution verified; NFR-6 absolute bar reported per §Q10)"

echo ""
msg 2 "  port.sh: PORT GATE GREEN (conformance + axiom audit + stackmeta + P9.3 fragment corpus + P5 gen drift + P6 pipeline + P7.1 harvest + P9 source surface + P10 automation)"
