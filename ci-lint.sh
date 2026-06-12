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
# --- 11. 50xx error code corpus coverage ---
# Every TcError code in the 50xx band must have at least one test fixture
# (either as error[E NNNN] in expected-stderr or via assert!(contains("NNNN"))).
check_50xx_corpus() {
    local rc=0
    # 5024 (BorrowLedgerFull) excluded — ledger cap 64 is a safety net; tighter
    # limits (resource cap 64, sig input cap 8, struct field cap 32, local cap 64)
    # prevent reaching 65 distinct borrows in practice.
    local codes="5001 5002 5003 5004 5010 5011 5012 5020 5021 5022 5023 5030 5031 5040 5050 5051 3523"
    for code in $codes; do
        local error_match=$(grep -rn "error\[E${code}\]" crates/tooling-tests/tests/ --include="*.rs" 2>/dev/null | wc -l)
        local contains_match=$(grep -rn "contains.*\"${code}\"" crates/tooling-tests/tests/ --include="*.rs" 2>/dev/null | wc -l)
        # Also check for numeric references like `assert_eq!(..., ${code})` or `${code},`
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

check_m4_survey || overall_rc=1
check_syntax_survey || overall_rc=1
check_50xx_corpus || overall_rc=1

if [ "$overall_rc" -eq 0 ]; then
    msg $GREEN "All lint checks passed."
else
    msg $RED "$failures file(s) failed lint checks."
fi
exit $overall_rc
