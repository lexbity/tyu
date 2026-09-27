#!/usr/bin/env bash
# PLAN-VERIFY-3 P7.1 tamper negative — MUTATE the Gen metadata.
#
# The Gen metadata (`tyu.gen/1`) is the harvest's binding surface. A tampered
# def name (a statement that does not exist in the kernel environment) must
# fail the harvest closed: nonzero exit + a `tyu.harvest-error/1` document
# naming the E6420 missing-statement condition — never a best-effort harvest.
#
# Operates IN the fixture dir (the fixture's lakefile resolves the port via
# `srcDir = "../.."`), mutating a backup of Tiny.gen.json and restoring on
# exit.
set -euo pipefail
HV="${1:?usage: run-mutate-gen.sh <harvest-fixture-dir>}"
cd "$HV"
[ -f Tiny.gen.json ] || { echo "FAIL: $HV/Tiny.gen.json missing" >&2; exit 1; }
[ -f lean-toolchain ] || { echo "FAIL: $HV/lean-toolchain missing (copy from the port)" >&2; exit 1; }
cp Tiny.gen.json .hvgen-backup.json
trap 'rm -f .hvgen-backup.json gen.v2.json gen.v2.json.audit.json' EXIT
python3 - .hvgen-backup.json .hvgen-tampered.json <<'PYEOF'
import json, sys
src, dst = sys.argv[1], sys.argv[2]
d = json.load(open(src))
d["statements"][0]["def"] = "stmt_Tiny_nonexistent_statement"
json.dump(d, open(dst, "w"), indent=1)
PYEOF
mv Tiny.gen.json .hvgen-backup2.json
mv .hvgen-tampered.json Tiny.gen.json
if TYU_HARVEST_GEN_DIR="$HV" TYU_HARVEST_OBL="$HV/Tiny.obl.json" \
     TYU_HARVEST_OUT="$HV/gen.v2.json" lake env lean "$HV/hvharvest.lean" >/dev/null 2>&1; then
    echo "FAIL: a mutated Gen metadata must fail the harvest (E6420)" >&2
    mv .hvgen-backup2.json Tiny.gen.json
    exit 1
fi
grep -q "E6420" "$HV/gen.v2.json"
mv .hvgen-backup2.json Tiny.gen.json
rm -f .hvgen-backup.json gen.v2.json.audit.json
echo "  tamper mutate-gen: harvest failed closed with an E6420 error document"