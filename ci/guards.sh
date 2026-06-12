#!/usr/bin/env bash
# Test-delivery guard: CI invariant layer (G1-G3).
#
# Checks:
#   G1  Every crate with #[test] runs >0 tests.
#   G2  No test=false / harness=false hides a #[test] outside tests/.
#   G3  (informational) Prints per-package executed-test counts.
#
# Escape hatch: add `# guards: allow-no-tests` as a comment in the
# package's Cargo.toml to suppress G1/G2 for that package.  This is
# itself greppable and must be reviewed when the structural issue is
# fixed.
#
# Usage:
#   bash ci/guards.sh
#     Exits 0 on pass, non-zero on failure.
#
# Mutation checks (manual, documented):
#   1. Add `test = false` to crates/ir/Cargo.toml -> guard MUST fail.
#   2. Gate all lmod tests behind a bogus feature -> guard MUST fail.
#   3. Revert both after check.

set -euo pipefail

RED=1; GREEN=2; YELLOW=3; NC=0
msg()  { local c=$1; shift; tput setaf "$c" 2>/dev/null || true; echo "  $*"; tput sgr0 2>/dev/null || true; }

failures=0
total_tests=0
counts=""

if [ ! -f Cargo.toml ] || ! grep -q '\[workspace\]' Cargo.toml 2>/dev/null; then
    msg $RED "Must be run from workspace root (no Cargo.toml with [workspace] found)"
    exit 1
fi

# Use python3 for JSON parsing (pre-installed on GitHub runners and most distros)
PYTHON=$(command -v python3 || command -v python || echo "")
if [ -z "$PYTHON" ]; then
    msg $RED "python3 is required but not found."
    exit 1
fi

# Get workspace metadata
metadata=$(cargo metadata --format-version 1 --no-deps 2>/dev/null) || {
    msg $RED "cargo metadata failed -- is the workspace valid?"
    exit 1
}

# Extract manifest paths
manifest_list=$("$PYTHON" -c "
import sys, json
data = json.load(sys.stdin)
for p in data['packages']:
    print(p['manifest_path'])
" <<< "$metadata" 2>/dev/null) || {
    msg $RED "Failed to parse cargo metadata"
    exit 1
}

while IFS= read -r manifest_path; do
    [ -z "$manifest_path" ] && continue
    pkg_dir=$(dirname "$manifest_path")
    pkg_name=$("$PYTHON" -c "
import sys, json
data = json.load(sys.stdin)
mp = '$manifest_path'
for p in data['packages']:
    if p['manifest_path'] == mp:
        print(p['name'])
        break
" <<< "$metadata" 2>/dev/null)

    local_src="$pkg_dir/src"
    local_tests="$pkg_dir/tests"

    # Check for escape hatch
    allow_no_tests=false
    if grep -q '# guards: allow-no-tests' "$manifest_path" 2>/dev/null; then
        allow_no_tests=true
    fi

    # Detect #[test] in source and test dirs (|| true keeps grep failure from
    # triggering set -e via pipefail inside $())
    has_test_anywhere=false
    has_test_in_source=false

    if [ -d "$local_src" ]; then
        count=$(grep -rl '#\[test\]' --include='*.rs' "$local_src" 2>/dev/null | head -1 | wc -l || true)
        if [ "$count" -gt 0 ]; then
            has_test_anywhere=true
            has_test_in_source=true
        fi
    fi
    if [ -d "$local_tests" ]; then
        count=$(grep -rl '#\[test\]' --include='*.rs' "$local_tests" 2>/dev/null | head -1 | wc -l || true)
        if [ "$count" -gt 0 ]; then
            has_test_anywhere=true
        fi
    fi

    if [ "$has_test_anywhere" = false ]; then
        counts="$counts|PKG $pkg_name: 0 tests (no #[test] found)"
        continue
    fi

    # --- G1: Zero-test guard ---
    # Sum test counts across ALL test targets (lib units, integration tests).
    # Uses the summary line "N tests, ..." from each target.
    # || true handles non-zero exit on compile failure.
    list_output=$(cargo test -p "$pkg_name" -- --list --color never 2>&1 || true)
    test_count=$(echo "$list_output" | awk '/^[0-9]+ tests,/ {total += $1} END {print total+0}' || echo "0")
    test_count=${test_count:-0}
    test_count=${test_count:-0}

    if [ "$test_count" -eq 0 ]; then
        if [ "$allow_no_tests" = true ]; then
            counts="$counts|PKG $pkg_name: 0 tests (ALLOWED: allow-no-tests)"
        else
            counts="$counts|PKG $pkg_name: 0 tests (FAIL)"
            msg $RED "  FAIL: $pkg_name has #[test] but runs 0 tests"
            failures=$((failures + 1))
        fi
    else
        total_tests=$((total_tests + test_count))
        counts="$counts|PKG $pkg_name: $test_count tests"
    fi

    # --- G2: test=false/harness=false with in-source #[test] ---
    # Only flag if there is a 'test = false' or 'harness = false' on a target
    # AND there is no [lib] target that hosts tests instead.
    if [ "$has_test_in_source" = true ]; then
        has_tests_via_lib=false
        if grep -q '^\[lib\]' "$manifest_path" 2>/dev/null; then
            # Extract the [lib] section and check for test=false (not just doctest=false)
            lib_section=$(awk '/^\[lib\]/{flag=1; next} /^\[/{flag=0} flag' "$manifest_path" 2>/dev/null || true)
            if ! echo "$lib_section" | grep -qE '(^|[^a-z])test\s*=\s*false'; then
                has_tests_via_lib=true
            fi
        fi

        if [ "$has_tests_via_lib" = false ]; then
            if grep -qE '^\s*(test|harness)\s*=\s*false' "$manifest_path" 2>/dev/null; then
                if [ "$allow_no_tests" = false ]; then
                    msg $RED "  FAIL: $pkg_name has test=false or harness=false but #[test] in source"
                    failures=$((failures + 1))
                fi
            fi
        fi
    fi
done < <(echo "$manifest_list")

echo ""
msg $GREEN "============================================"
msg $GREEN "Per-package test counts:"
echo "$counts" | tr '|' '\n' | sort | while IFS= read -r line; do
    case "$line" in
        *FAIL*)        msg $RED "$line" ;;
        *ALLOWED*)     msg $YELLOW "$line" ;;
        PKG*)          msg $GREEN "$line" ;;
    esac
done
echo ""
msg $GREEN "Total workspace tests (default features): $total_tests"

if [ "$failures" -gt 0 ]; then
    msg $RED "FAILED: $failures package(s) have test-delivery issues"
    exit 1
fi
msg $GREEN "All guard checks passed."
