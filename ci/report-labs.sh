#!/usr/bin/env bash
# PLAN-RELEASE-1 S11 (report leg; consumed by S9's release summary) — the
# lab-regression status line for release draft notes (FR-24, Q11).
#
# Given a commit SHA, query the most recent `labs.yml` run for that SHA and
# emit ONE markdown line:
#   Labs: green (<widget>): <run url>       — latest labs.yml run concluded success
#   Labs: RED (<widget>): <run url>         — latest labs.yml run failed
#   Labs: no run for SHA <sha>              — labs.yml absent, or never ran on this SHA
#
# ABSENCE is not failure: a release cut before the lab workflow exists (or
# with no run for its SHA) reports the "no run" line and exits 0. This script
# is NEVER a gate — it feeds release draft notes only (Q11: the book does not
# gate releases).
#
# The per-fixture count rendering ("82/82") arrives when the S11 lab runner
# (ci/run-labs.sh) lands; until then the workflow CONCLUSION (green/red) is
# the honest machine signal — a red lab run IS shown in the draft notes.
#
# Usage: ci/report-labs.sh <commit-sha>
set -euo pipefail

SHA="${1:-}"
if [ -z "$SHA" ]; then
    echo "usage: ci/report-labs.sh <commit-sha>" >&2
    exit 2
fi

if ! command -v gh >/dev/null 2>&1 || ! gh auth status >/dev/null 2>&1; then
    echo "Labs: not reported (gh not authenticated)"
    exit 0
fi

# `gh run list` errors (stderr) when the workflow file does not exist — that
# is the "not yet wired" case, reported as absence, never as a failure.
RUN_JSON="$(gh run list --workflow=labs.yml --commit "$SHA" --limit 1 \
    --json databaseId,conclusion,htmlUrl 2>/dev/null || true)"
ID="$(printf '%s' "$RUN_JSON" | jq -r '.[0].databaseId // empty' 2>/dev/null || true)"
if [ -z "$ID" ]; then
    echo "Labs: no run for SHA $SHA (lab regression workflow has not run on this commit)"
    exit 0
fi
CONCLUSION="$(printf '%s' "$RUN_JSON" | jq -r '.[0].conclusion // "unknown"')"
URL="$(printf '%s' "$RUN_JSON" | jq -r '.[0].htmlUrl')"
case "$CONCLUSION" in
    success) echo "Labs: GREEN (labs.yml run #$ID succeeded): $URL" ;;
    "") echo "Labs: PENDING (labs.yml run #$ID): $URL" ;;
    *) echo "Labs: RED (labs.yml run #$ID concluded $CONCLUSION): $URL" ;;
esac
exit 0