#!/usr/bin/env bash
# Local build-artifact cleanup — reclaim disk after test/check sessions.
#
# CI runners are ephemeral; this is a developer tool for the checkout, where
# build outputs accumulate across toolchain bumps and gate runs:
#
#   --quick   churn artifacts that regenerate on the next gate run, keeping
#             the main builds warm: the ci/wasm.sh gate logs under /tmp, the
#             isolated target/g52-prod verifier build, and the ci/
#             docker-audit.sh rehearsal container.
#   --deep    everything above, plus full build outputs: the cargo target
#             dir (debug + release + wasm32), fuzz/target, the Lean port
#             build outputs (verification/ports/*/.lake). A `cargo build`
#             afterwards starts from scratch.
#   --all     everything above, plus regenerable caches: web node_modules
#             (restore with `npm ci`), the .tyu-verify verdicts cache
#             (§7.4), and the playwright browser cache (~1.3 GiB,
#             re-downloaded on the next browser smoke).
#
# With NO argument the script is a DRY RUN: it prints what each tier would
# remove and the reclaimable size. It deletes nothing.
#
# Usage:
#   bash ci/clean.sh            dry run — show what each tier would remove
#   bash ci/clean.sh --quick    logs + isolated dirs + rehearsal container
#   bash ci/clean.sh --deep     + full target/, fuzz/target, Lean outputs
#   bash ci/clean.sh --all      + node_modules, .tyu-verify, playwright cache
#
# Docker IMAGES are deliberately left alone (ubuntu:26.04 is reused by
# ci/docker-audit.sh); reclaim those with `docker image prune` if wanted.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

mode="dry-run"
case "${1:-}" in
    "")       mode="dry-run" ;;
    --quick)  mode="quick" ;;
    --deep)   mode="deep" ;;
    --all)    mode="all" ;;
    -h|--help)
        sed -n '2,30p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    *)
        echo "clean: unknown argument '${1}' (dry-run | --quick | --deep | --all)" >&2
        exit 2
        ;;
esac

# Tier ranks: an item is deleted when its tier rank is <= the selected mode's.
tier_rank() {
    case "$1" in
        quick) echo 1 ;;
        deep)  echo 2 ;;
        all)   echo 3 ;;
    esac
}
active_rank=0
[ "$mode" = "quick" ] && active_rank=1
[ "$mode" = "deep" ]  && active_rank=2
[ "$mode" = "all" ]   && active_rank=3

sizeof() { du -sch "$@" 2>/dev/null | tail -1 | cut -f1; }
# Expand an item path to existing filesystem entries, one per line. Glob
# entries (the /tmp gate logs) expand to their matches; plain paths pass
# through only when they exist. Matches are always passed on QUOTED by the
# caller, so paths with spaces stay safe.
#
# The glob test deliberately uses [[ == ]] rather than `case $p in *\*)`:
# bash 5.3's case matching treats a `*` IN THE SUBJECT as a wildcard, so a
# pattern requiring a literal asterisk (the very thing being detected here)
# can never match. `[[ == ]]` keeps the documented semantics.
expand() {
    if [[ "$1" == *"*"* ]]; then
        compgen -G "$1" 2>/dev/null || true
    elif [ -e "$1" ]; then
        printf '%s\n' "$1"
    fi
}

# add <tier> <path-or-:docker:name> [description]
items=()
add() { items+=("$1|$2|${3:-$2}"); }

# --- quick tier: churn from the last gate run
add quick "/tmp/g5*.log"                     "ci/wasm.sh gate logs"
add quick "/tmp/wasm-repl-build.log"         "wasm-repl build log"
add quick "/tmp/verifier-prod-build.log"     "verifier prod-build log"
add quick "/tmp/tyu-docker-audit*.log"       "docker-audit logs (superseded)"
add quick "/tmp/docker-audit*.log"           "docker-audit logs"
add quick "$TARGET_DIR/g52-prod"             "isolated verifier build (G52)"
add quick "$TARGET_DIR/tmp"                  "cargo scratch dir"
add quick ":docker:tyu-audit"                "ci/docker-audit.sh rehearsal container"

# --- deep tier: full build outputs (rebuilt by cargo/lake)
add deep "$TARGET_DIR"                       "cargo target dir (debug/release/wasm32)"
add deep "$ROOT/fuzz/target"                 "cargo-fuzz target dir"
add deep "$ROOT"/verification/ports/*/.lake  "Lean port build outputs"

# --- all tier: regenerable caches (npm ci / re-verify / re-download)
add all "$ROOT/web/test/node_modules"        "web/test deps (npm ci restores)"
add all "$ROOT/web/editor/node_modules"      "web/editor deps (npm ci restores)"
add all "$ROOT/.tyu-verify"                  "verdicts cache (re-verified on next build)"
add all "${PLAYWRIGHT_BROWSERS_PATH:-$HOME/.cache/ms-playwright}" \
    "playwright browsers (re-downloaded on next smoke)"

removed_any=0
dry_run_total=0
for entry in "${items[@]}"; do
    tier="${entry%%|*}"; rest="${entry#*|}"
    path="${rest%%|*}"; desc="${rest#*|}"
    # Dry run lists EVERY tier; a delete mode only acts on its own tier and
    # below (--all implies --deep implies --quick).
    if [ "$mode" != "dry-run" ]; then
        [ "$(tier_rank "$tier")" -le "$active_rank" ] || continue
    fi

    if [ "$path" = ":docker:tyu-audit" ]; then
        if docker container inspect "${path#*:docker:}" > /dev/null 2>&1; then
            if [ "$mode" = "dry-run" ]; then
                echo "  [$tier] would remove: $desc"
            else
                docker rm -f "${path#*:docker:}" > /dev/null 2>&1 || true
                echo "  [$tier] removed: $desc"
            fi
            removed_any=1
        fi
        continue
    fi

    local_matches=()
    mapfile -t local_matches < <(expand "$path")
    [ "${#local_matches[@]}" -gt 0 ] || continue
    if [ "$mode" = "dry-run" ]; then
        echo "  [$tier] would remove: $desc — $(sizeof "${local_matches[@]}")"
        dry_run_total=$((dry_run_total + $(du -sb "${local_matches[@]}" 2>/dev/null | tail -1 | cut -f1)))
    else
        rm -rf "${local_matches[@]}"
        echo "  [$tier] removed: $desc"
    fi
    removed_any=1
done

echo ""
if [ "$mode" = "dry-run" ]; then
    if [ "$dry_run_total" -gt 0 ]; then
        echo "clean (dry run): $(numfmt --to=iec --suffix=B "$dry_run_total" 2>/dev/null || echo "$dry_run_total bytes") reclaimable in total — rerun with --quick, --deep or --all to delete."
    else
        echo "clean (dry run): nothing to reclaim."
    fi
elif [ "$removed_any" -eq 0 ]; then
    echo "clean ($mode): nothing to remove."
else
    echo "clean ($mode): done."
fi
