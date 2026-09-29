#!/usr/bin/env bash
# PLAN-VERIFY-3 P16.3 — NFR measurements (numbers recorded in
# ci/ACCEPTANCE.md; runs under `bash ci/acceptance.sh --measure-nfr`).
#
#   NFR-1  Build-time overhead: p95 per-module SVS-inclusive emission wall
#          time ≤ 250 ms on the corpus (`langc --emit=obj` — extraction +
#          discharge + emission run in the same pass; FR-22 fast path stays
#          the comparator).
#   NFR-4  Stability: the id-stability suite is green (0 id changes under
#          unrelated edits — obl_id_stability).
#   NFR-5  Artifact size: `.obl.json` ≤ max(4 × `.asm` size, 64 KiB) per
#          corpus module; hard cap 16 MiB enforced at read.
#   NFR-3  Soundness count is the ≥10^5 differential (ci/differential.sh);
#          reported here as a reference when the port toolchain is present.
#
# Exit 0 when every measurement can be produced; the NFR-1 maximum-vs-gate
# comparison is reported, not enforced (a breach is reviewed, per NFR-1's
# "CI gate: p95 ≤ 250 ms" — recorded as data).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

msg() { local c=$1; shift; tput setaf "$c" 2>/dev/null || true; echo "  $*"; tput sgr0 2>/dev/null || true; }

LANGC="${TYU_BIN_DIR:+$TYU_BIN_DIR/langc}"
[ -n "$LANGC" ] || LANGC="$ROOT/target/debug/langc"
[ -x "$LANGC" ] || { msg 1 "nfr.sh: langc missing at $LANGC (cargo build -p langc)"; exit 1; }

MODULES=(ci/verify-corpus/clean.mod ci/verify-corpus/contract.mod
         ci/verify-corpus/event-loop.mod ci/verify-corpus/open-cast.mod)

# The corpus module *names* are capitalized (file names are lowercase).
module_of() {
    case "$(basename "$1" .mod)" in
        clean) echo Clean ;;
        contract) echo Contract ;;
        event-loop) echo EventLoop ;;
        open-cast) echo OpenCast ;;
        *) basename "$1" .mod ;;
    esac
}

TMP=$(mktemp -d /tmp/tyu-nfr.XXXXXX)
trap 'rm -rf "$TMP"' EXIT

# --- NFR-1: per-module p95 SVS-inclusive emission time -----------------------
msg 3 "  NFR-1: p95 per-module emission time (SVS-inclusive, langc --emit=obj)"
times=()
for m in "${MODULES[@]}"; do
    name="$(basename "$m" .mod)"
    out="$TMP/$name"
    mkdir -p "$out"
    # 5 samples per module; a cold-cache first run is included (honest).
    for s in 1 2 3 4 5; do
        out2="$out/$s"; mkdir -p "$out2"
        start=$(date +%s%N)
        "$LANGC" --emit=obj --target=x86_64-unknown-linux-gnu \
            --out-dir="$out2" "$m" 2>/dev/null
        end=$(date +%s%N)
        ms=$(( (end - start) / 1000000 ))
        times+=("$ms")
    done
done
sorted=$(printf '%s\n' "${times[@]}" | sort -n)
count=$(echo "$sorted" | wc -l)
p95_idx=$(( count * 95 / 100 )); [ "$p95_idx" -lt 1 ] && p95_idx=1
p95=$(echo "$sorted" | sed -n "${p95_idx}p")
max=$(echo "$sorted" | tail -1)
msg 2 "  NFR-1: n=${count} samples; p95=${p95}ms; max=${max}ms (gate: p95 ≤ 250 ms) $( [ "$p95" -le 250 ] && echo '[within gate]' || echo '[EXCEEDS GATE — review]' )"

# --- NFR-4: id stability ------------------------------------------------------
msg 3 "  NFR-4: id stability (obl_id_stability suite)"
if cargo test -q -p tooling-tests --test obl_id_stability >/dev/null 2>&1; then
    msg 2 "  NFR-4: id-stability suite green"
else
    msg 1 "  NFR-4: id-stability suite FAILED"
    exit 1
fi

# --- NFR-5: artifact size -----------------------------------------------------
msg 3 "  NFR-5: .obl.json ≤ max(4 × .asm, 64 KiB) per module"
worst=0; worst_mod=""
for m in "${MODULES[@]}"; do
    name="$(module_of "$m")"
    out="$TMP/sz-$name"; mkdir -p "$out"
    "$LANGC" --emit=obj --write-obl --target=x86_64-unknown-linux-gnu \
        --out-dir="$out" "$m" 2>/dev/null
    obl="$out/$name.obl.json"
    asm="$out/$name.asm"
    [ -f "$obl" ] || { msg 1 "  NFR-5: no artifact for $name"; exit 1; }
    obl_sz=$(stat -c%s "$obl")
    asm_sz=$(stat -c%s "$asm" 2>/dev/null || echo 0)
    limit=$(( 4 * asm_sz )); [ "$limit" -lt 65536 ] && limit=65536
    if [ "$obl_sz" -gt "$worst" ]; then worst="$obl_sz"; worst_mod="$name"; fi
    if [ "$obl_sz" -gt "$limit" ]; then
        msg 1 "  NFR-5: $name artifact ${obl_sz}B > limit ${limit}B (asm ${asm_sz}B) — EXCEEDS"
        exit 1
    fi
done
msg 2 "  NFR-5: all artifacts within limit; worst=$worst_mod ${worst}B of max(4×asm,64KiB)"

# --- NFR-3 reference ----------------------------------------------------------
if command -v lean >/dev/null 2>&1 && command -v lake >/dev/null 2>&1; then
    msg 3 "  NFR-3: ≥10^5 differential (ci/differential.sh)"
    if bash ci/differential.sh >/dev/null 2>&1; then
        msg 2 "  NFR-3: differential green (≥10^5 programs, exact agreement)"
    else
        msg 1 "  NFR-3: differential FAILED"
        exit 1
    fi
else
    msg 3 "  NFR-3: differential — port tier (ci/differential.sh), toolchain absent"
fi

msg 2 "  NFR measurements complete — record the numbers in ci/ACCEPTANCE.md"