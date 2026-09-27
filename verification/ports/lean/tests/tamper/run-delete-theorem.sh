#!/usr/bin/env bash
# PLAN-VERIFY-3 P7.1 tamper negative — DELETE the `obl_*` theorems.
#
# A statement with no theorem is `open` (unproven is a state, not a fault):
# the harvest must succeed (exit 0) with every obligation recorded `open`
# and witness reason `unproven` — never a crash, never a fabricated proof.
#
# Operates IN the fixture dir (the fixture's lakefile resolves the port via
# `srcDir = "../.."`), editing a backup of TinyFix.lean and restoring on
# exit — the same discipline the sorry negative in ci/port.sh uses.
set -euo pipefail
HV="${1:?usage: run-delete-theorem.sh <harvest-fixture-dir>}"
cd "$HV"
[ -f TinyFix.lean ] || { echo "FAIL: $HV/TinyFix.lean missing" >&2; exit 1; }
[ -f lean-toolchain ] || { echo "FAIL: $HV/lean-toolchain missing (copy from the port)" >&2; exit 1; }
cp TinyFix.lean .hvdel-backup.lean
trap 'rm -f .hvdel-backup.lean del.v2.json; mv .hvdel-built.lean TinyFix.lean 2>/dev/null || true' EXIT
python3 - .hvdel-backup.lean .hvdel-built.lean <<'PYEOF'
import re, sys
src, dst = sys.argv[1], sys.argv[2]
text = open(src).read()
new = re.sub(r"(?ms)^theorem obl_Tiny_inc_subtype_range_\d\s*:\s*stmt_Tiny_inc_subtype_range_\d\s*:=\s*by\s*\n\s*trivial\s*\n\n?", "", text)
assert "theorem obl_Tiny" not in new, "theorem strip failed — fixture shape changed"
open(dst, "w").write(new)
PYEOF
mv TinyFix.lean .hvdel-backup2.lean
mv .hvdel-built.lean TinyFix.lean
lake build TinyFix >/dev/null 2>&1
TYU_HARVEST_GEN_DIR="$HV" TYU_HARVEST_OBL="$HV/Tiny.obl.json" \
  TYU_HARVEST_OUT="$HV/del.v2.json" lake env lean "$HV/hvharvest.lean" >/dev/null 2>&1
python3 - "$HV/del.v2.json" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
assert d["verdicts"], "deleted theorems leave obligation records"
for r in d["verdicts"]:
    assert r["status"] == "open", f"deleted theorem must harvest open, got {r}"
    assert r.get("witness", {}).get("reason") == "unproven", r
print("  tamper delete-theorem: every obligation open (unproven is a state, not a fault)")
PYEOF
mv .hvdel-backup2.lean TinyFix.lean
rm -f .hvdel-backup.lean del.v2.json