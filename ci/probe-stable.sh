#!/usr/bin/env bash
# PLAN-RELEASE-1 S3 / FR-19 — non-blocking stable-channel probe.
#
# Attempts a stable-channel build of the host binary set and records the
# verdict (and the first error when it fails) into the GitHub step summary.
# The probe NEVER fails the job — exit 0 either way — so a red stable build
# is a measured signal, not a CI block. Two consecutive green weekly probes
# are the promotion signal (default pin -> stable), a maintainer decision
# with a release-notes notice, never a silent migration.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

BINARIES=(-p langc -p tyu -p lmod-pack -p lmod-sign -p lmod-encrypt)

bash "$ROOT/ci/retry.sh" -- rustup toolchain install stable --profile minimal >/dev/null

log="$(mktemp)"
if cargo +stable build --release "${BINARIES[@]}" >"$log" 2>&1; then
    verdict="STABLE-PROBE: pass"
else
    first_err="$(grep -m1 -E 'error(\[E[0-9]+\])?:|could not compile' "$log" || true)"
    [ -n "$first_err" ] || first_err="(no error line captured)"
    verdict="STABLE-PROBE: fail — ${first_err}"
fi
rm -f "$log"

summary="${GITHUB_STEP_SUMMARY:-}"
if [ -n "$summary" ]; then
    {
        echo "### Stable toolchain probe (non-blocking, FR-19)"
        echo ""
        echo "\`$verdict\`"
    } >>"$summary"
fi

echo "$verdict"
exit 0