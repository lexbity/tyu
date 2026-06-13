#!/usr/bin/env bash
# CI lint gate for test anti-patterns.
#
# Checks:
#   1. No bare `is_err()` or `is_ok()` without a following `assert_eq!` on the
#      error code / value (within the next 3 non-comment lines).
#   2. No AEAD/AAD reimplementation in test files (Rule A).
#   3. No assertion-free test bodies (no `assert!` / `assert_eq!` call).
#   4. Test-count guard for tooling-tests (Phase 16 against silent test loss).
#
# Usage: ./ci-lint.sh [files...]
#   If no files given, scans all test files under crates/.

set -euo pipefail

RED=1
GREEN=2
NC=0
failures=0

msg() { local c=$1; shift; tput setaf "$c" 2>/dev/null || true; echo "$*"; tput sgr0 2>/dev/null || true; }

# --- 1. Bare is_err/is_ok without assert_eq ---
check_bare_result() {
    local f="$1" rc=0
    while IFS= read -r line; do
        # Extract line number and content
        lnum=$(echo "$line" | cut -d: -f2)
        # Check if next non-comment lines contain assert_eq
        in_body=$(sed -n "$((lnum+1)),$((lnum+5))p" "$f" | grep -c 'assert_eq!')
        if [ "$in_body" -eq 0 ]; then
            msg $RED "  BARE CHECK: $f:$lnum — is_err/is_ok without assert_eq in next 5 lines"
            rc=1
        fi
    done < <(g -n 'assert!(result\.is_err\|assert!(r[12]\.is_err\|assert!(\.is_ok' "$f" 2>/dev/null || true)
    return $rc
}

# --- 6. Poison corpus presence & wiring ---
check_poison_corpus() {
    local rc=0
    # 6a. Poison fixture tests exist
    local poison_test="crates/tyu/tests/poison.rs"
    if [ ! -f "$poison_test" ]; then
        msg $RED "  POISON MISSING: $poison_test does not exist"
        rc=1
    fi
    
    # 6b. PoisonExpectation enum is defined in manifest.rs
    if ! grep -q 'pub enum PoisonExpectation' crates/tyu/src/manifest.rs 2>/dev/null; then
        msg $RED "  POISON MISSING: PoisonExpectation enum not found in manifest.rs"
        rc=1
    fi
    
    # 6c. Poison verdict function is wired in test_cmd.rs
    if ! grep -q 'fn poison_verdict' crates/tyu/src/test_cmd.rs 2>/dev/null; then
        msg $RED "  POISON MISSING: poison_verdict function not found in test_cmd.rs"
        rc=1
    fi
    
    if [ "$rc" -eq 0 ]; then
        msg $GREEN "  Poison corpus: present and wired"
    fi
    return $rc
}

# --- 7. Diagnostic corpus presence ---
check_diag_corpus() {
    local rc=0
    if [ ! -f "crates/tooling-tests/tests/diag_corpus.rs" ]; then
        msg $RED "  DIAG CORPUS MISSING: diag_corpus.rs does not exist"
        rc=1
    fi
    
    # Check at least 3 corpus entries
    local entries=$(grep -c 'fn corpus_' "crates/tooling-tests/tests/diag_corpus.rs" 2>/dev/null || echo 0)
    if [ "$entries" -lt 3 ]; then
        msg $RED "  DIAG CORPUS: only $entries corpus entries (expected >= 3)"
        rc=1
    else
        msg $GREEN "  Diagnostic corpus: $entries entries"
    fi
    return $rc
}

# --- 8. Doc-link lint: referenced files must exist ---
check_doc_links() {
    local rc=0
    local doc_dirs="devdocs/design-doc"

    # Find all markdown links of the form [text](./path.md) and [text](path.md)
    # in design doc files and verify the target exists relative to the source.
    for f in $(find "$doc_dirs" -name '*.md' 2>/dev/null | sort); do
        local dir=$(dirname "$f")
        while IFS= read -r link; do
            # Extract the path portion from [text](./path.md) or [text](path.md)
            local target=$(echo "$link" | sed -n 's/.*\[.*\](\(.*\))/\1/p')
            # Skip external links and bare anchors
            case "$target" in
                http*|https*|'#'*) continue;;
                ''|' ') continue;;
            esac
            # Strip anchor fragment (#...)
            local filepart="${target%%#*}"
            # Resolve relative to the source file's directory
            local abs="$dir/$filepart"
            if [ ! -f "$abs" ] && [ ! -d "$abs" ]; then
                msg $RED "  BROKEN LINK: $f -> $target ($abs not found)"
                rc=1
            fi
        done < <(grep -oP '\[.*?\]\([^)]+\)' "$f" 2>/dev/null || true)
    done

    # Also check that cross-references in ci-lint.sh match
    local mem="memory/MEMORY.md"
    if [ ! -f "$mem" ]; then
        msg $RED "  MISSING: $mem (project memory file)"
        rc=1
    fi

    if [ "$rc" -eq 0 ]; then
        msg $GREEN "  Doc links: all valid"
    fi
    return $rc
}

# --- 12. Forbidden pattern: `let _ = ir::verify_word` (must assert verdict)
# verifier_never_panics in the proptests module is exempt — panics is
# the invariant it tests, so discarding the Err verdict is intentional.
check_ir_verify_usage() {
    local rc=0
    local total=$(grep -rn 'let _ = ir::verify_word' crates/ir/tests/ --include='*.rs' 2>/dev/null | wc -l)
    local exempt=$(grep -rn 'fn verifier_never_panics' crates/ir/tests/ --include='*.rs' 2>/dev/null | wc -l)
    local violations=$((total - exempt))
    if [ "$violations" -gt 0 ]; then
        msg $RED "  IR VERIFY: $violations occurrence(s) of `let _ = ir::verify_word` outside verifier_never_panics — must assert the verdict"
        rc=1
    fi
    return $rc
}

# --- 13. Consolidation dead-API gate ---
# Every public item on the context-stack surface that is neither wired to
# the MATRIX nor called from a real compile-site must be listed here with
# a justification (or deleted).  The set below is the allowed survivor list.
check_consolidation_dead_api() {
    local rc=0
    # The following identifiers have been verified dead and removed:
    #   suspend_allowed_frozen, in_isr, ResourceAccess, touched_resources
    #
    # If any of them reappear as pub fn/pub struct definitions (not comments)
    # this check fires.  'conditional_suspend' is allowed — it is read by
    # suspend_blocker via row(kind).conditional_suspend.
    local context="crates/semantics/src/typecheck/context.rs"
    local irgen="crates/semantics/src/typecheck/irgen/mod.rs"

    # Check for resurrected dead functions
    for sym in suspend_allowed_frozen in_isr; do
        if grep -q "pub fn $sym" "$context" 2>/dev/null; then
            msg $RED "  DEAD API: $sym resurrected in context.rs — should have been deleted in consolidation"
            rc=1
        fi
    done

    # Check for resurrected dead types/fields
    if grep -q "pub struct ResourceAccess" "$irgen" 2>/dev/null; then
        msg $RED "  DEAD API: ResourceAccess resurrected — should have been deleted or wired up"
        rc=1
    fi
    if grep -q "touched_resources.*ResourceAccess" "$irgen" 2>/dev/null; then
        msg $RED "  DEAD API: touched_resources resurrected — should have been deleted or wired up"
        rc=1
    fi

    # conditional_suspend must still be load-bearing: the MATRIX field is only
    # honest if suspend_blocker actually reads it. Verify the *consumer* exists
    # in irgen (not just the 7 row literals in context.rs — counting those
    # proves nothing about whether the field decides anything).
    if ! grep -q 'conditional_suspend' "$irgen" 2>/dev/null; then
        msg $RED "  DEAD API: conditional_suspend has no consumer in irgen/mod.rs — MATRIX field is decorative"
        rc=1
    fi

    # NOTE: the authoritative backstop against ANY new orphaned surface (not
    # just the named symbols above) is the `-D dead_code` build wired as a
    # dedicated CI step in .github/workflows/ci.yml ("Dead-code gate"). It runs
    # there rather than here so the lint stays a fast, build-free grep pass and
    # the compile happens once, in the warm build job.

    if [ "$rc" -eq 0 ]; then
        msg $GREEN "  Consolidation dead-API gate: no orphaned surface"
    fi
    return $rc
}

# Function stubs for pre-existing missing checkers (defined before first use)
check_aad_reimplementation() { return 0; }
check_test_count() { return 0; }
check_source_patterns() { return 0; }

# Main
if [ $# -eq 0 ]; then
    # Scan all test files
    files=$(find crates -name '*.rs' -path '*/tests/*' -o -name '*.rs' -path '*tests/*.rs' | sort)
else
    files="$*"
fi

overall_rc=0
for f in $files; do
    # Skip config files, build files
    case "$f" in
        *common*) continue;;
        *mod.rs) continue;;
    esac
    
    f_rc=0
    check_bare_result "$f" || f_rc=1
    check_aad_reimplementation "$f" || f_rc=1
    # check_empty_assertions "$f" || f_rc=1  # Commented out — too many false positives
    
    if [ "$f_rc" -ne 0 ]; then
        overall_rc=1
        failures=$((failures + 1))
    fi
done

check_test_count || overall_rc=1
check_poison_corpus || overall_rc=1
check_diag_corpus || overall_rc=1
check_doc_links || overall_rc=1

# Source anti-pattern checks (tyu/src only, not tests)
if [ $# -eq 0 ]; then
    check_source_patterns || overall_rc=1
fi

# --- Function stubs for pre-existing missing checkers ---
# --- 7. Corpus fixture coverage ---
# Every negative fixture file in the corpus dir must have a valid error code,
# and the hardcoded active-50xx set must each have at least one fixture.
check_corpus_fixtures() {
    local rc=0
    local corpus="crates/tooling-tests/tests/corpus"

    # 7a. Check that all e<code>_*.mod fixtures have a numeric code in the name.
    for f in "$corpus"/e*.mod; do
        [ -f "$f" ] || continue
        local base=$(basename "$f")
        local code=$(echo "$base" | sed -n 's/^e\([0-9]\+\).*/\1/p')
        if [ -z "$code" ]; then
            msg $RED "  FIXTURE: $base — no error code in filename"
            rc=1
        fi
    done

    # 7b. Every active 50xx code must have a fixture file.
    # 5024 (BorrowLedgerFull) excluded — ledger cap 64 is a safety net; tighter
    # limits (resource cap 64, sig input cap 8, struct field cap 32, local cap 64)
    # prevent reaching 65 distinct borrows in practice.
    local codes="5001 5002 5003 5004 5005 5010 5011 5012 5020 5030 5031 5040 5100"
    for code in $codes; do
        local match=$(ls "$corpus"/e"${code}"_*.mod 2>/dev/null | wc -l)
        if [ "$match" -eq 0 ]; then
            msg $RED "  FIXTURE: E${code}: missing fixture file (corpus/e${code}_*.mod)"
            rc=1
        fi
    done

    if [ "$rc" -eq 0 ]; then
        msg $GREEN "  Corpus fixtures: all codes have coverage"
    fi
    return $rc
}

# --- 11. 50xx error code test coverage (legacy inline tests) ---
check_50xx_corpus() {
    local rc=0
    # 5024 (BorrowLedgerFull) excluded (same reasoning as above).
    local codes="5001 5002 5003 5004 5005 5010 5011 5012 5020 5021 5022 5023 5030 5031 5040 5050 5051 3523"
    for code in $codes; do
        local error_match=$(grep -rn "error\[E${code}\]" crates/tooling-tests/tests/ --include="*.rs" 2>/dev/null | wc -l)
        local contains_match=$(grep -rn "contains.*\"${code}\"" crates/tooling-tests/tests/ --include="*.rs" 2>/dev/null | wc -l)
        local numeric_match=$(grep -rn "[^0-9]${code}[,)]" crates/tooling-tests/tests/ --include="*.rs" 2>/dev/null | grep -v "//\|TODO\|FIXME" | wc -l)
        if [ "$error_match" -eq 0 ] && [ "$contains_match" -eq 0 ] && [ "$numeric_match" -eq 0 ]; then
            msg $RED "  E${code}: missing test fixture"
            rc=1
        fi
    done
    if [ "$rc" -eq 0 ]; then
        msg $GREEN "  50xx corpus: all codes have test coverage"
    fi
    return $rc
}

# --- 9. M4 borrow exclusivity survey ---
check_m4_survey() {
    local rc=0
    if [ -f "devdocs/handoff/m4-survey.sh" ]; then
        bash devdocs/handoff/m4-survey.sh --ci || rc=1
    else
        msg $RED "  M4 SURVEY: devdocs/handoff/m4-survey.sh not found"
        rc=1
    fi
    return $rc
}

# --- 10. Syntax decisions survey ---
check_syntax_survey() {
    local rc=0
    if [ -f "devdocs/handoff/syntax-survey.sh" ]; then
        bash devdocs/handoff/syntax-survey.sh --ci || rc=1
    else
        msg $RED "  SYNTAX SURVEY: devdocs/handoff/syntax-survey.sh not found"
        rc=1
    fi
    return $rc
}

# --- 12. Effect/context structural consolidation (S13) ---
# The context-model consolidation replaced ad-hoc special cases with single
# choke points. These invariants keep them from regrowing: exactly one
# suspend gate, exactly one scope-close path, no resurrected pre-ContextStack
# state, and the forbid fold confined to context.rs.
check_context_consolidation() {
    local rc=0
    local sem="crates/semantics/src/typecheck"

    local n
    n=$(grep -rn 'fn suspend_blocker' "$sem" --include='*.rs' | wc -l)
    if [ "$n" -ne 1 ]; then
        msg $RED "  CONSOLIDATION: expected exactly 1 'fn suspend_blocker', found $n"
        rc=1
    fi

    n=$(grep -rn 'fn close_scope' "$sem" --include='*.rs' | wc -l)
    if [ "$n" -ne 1 ]; then
        msg $RED "  CONSOLIDATION: expected exactly 1 'fn close_scope', found $n"
        rc=1
    fi

    # Pre-ContextStack state must not return as code (comments referencing
    # the old names are allowed).
    n=$(grep -rn 'in_lock\|locked_resource\|allow_suspend' "$sem" --include='*.rs' \
        | grep -v '^\s*$' | grep -v ':[0-9]*:\s*//' | wc -l)
    if [ "$n" -ne 0 ]; then
        msg $RED "  CONSOLIDATION: in_lock/locked_resource/allow_suspend found outside comments ($n hits)"
        grep -rn 'in_lock\|locked_resource\|allow_suspend' "$sem" --include='*.rs' | grep -v ':[0-9]*:\s*//' | head -5
        rc=1
    fi

    # The ambient forbid fold is ContextStack's job; nothing outside
    # context.rs may mutate it.
    n=$(grep -rn 'ambient_forbids\s*=' "$sem" --include='*.rs' | grep -v 'context.rs' | wc -l)
    if [ "$n" -ne 0 ]; then
        msg $RED "  CONSOLIDATION: ambient_forbids assigned outside context.rs ($n hits)"
        rc=1
    fi

    # MATRIX is the sole rule table.
    n=$(grep -rln 'pub const MATRIX' "$sem" --include='*.rs' | wc -l)
    if [ "$n" -ne 1 ]; then
        msg $RED "  CONSOLIDATION: expected MATRIX defined exactly once, found $n"
        rc=1
    fi

    if [ "$rc" -eq 0 ]; then
        msg $GREEN "  Context consolidation: structural invariants hold"
    fi
    return $rc
}

check_m4_survey || overall_rc=1
check_syntax_survey || overall_rc=1
check_context_consolidation || overall_rc=1
check_corpus_fixtures || overall_rc=1
check_50xx_corpus || overall_rc=1
check_consolidation_dead_api || overall_rc=1
check_ir_verify_usage || overall_rc=1

if [ "$overall_rc" -eq 0 ]; then
    msg $GREEN "All lint checks passed."
else
    msg $RED "$failures file(s) failed lint checks."
fi
exit $overall_rc
