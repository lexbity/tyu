#!/usr/bin/env bash
# PLAN-RELEASE-1 S9 — release-tag verification (FR-18 gate step 1 / FR-7).
#
# Validates a release tag before (and independent of) the heavy gate jobs:
#   1. the tag is the canonical `vMAJOR.MINOR.0` form (FR-7 — real patch tags
#      are forbidden; the git SHA is the patch identity, §Q2), and
#   2. the tagged tree's CHANGELOG.md carries the section for that version
#      (cut-release.sh refuses to mint without it — this re-checks the pushed
#      ref because a tag pushed from a dirty/unprepared tree is a real risk),
#   3. REPORTS (never gates) a local `git verify-tag` against the committed
#      signing key — GitHub's Verified badge is the primary signature signal
#      and CI holds no private key (Q6).
#
# Sole implementation of the tag-format check: invoked by release.yml's
# `verify` job and by guards.sh G55 on v* tag refs, so the regex cannot drift
# between the fail-fast gate and the repo-invariant guard.
#
# Exit 0 on a canonical, changelog-recorded tag (signature REPORTED only);
# exit 2 on a malformed tag or a missing changelog section.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TAG="${1:-}"
if [ -z "$TAG" ]; then
    echo "usage: ci/release-verify-tag.sh vMAJOR.MINOR.0" >&2
    exit 2
fi

if ! [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.0$ ]]; then
    echo "error: tag '$TAG' is not canonical vMAJOR.MINOR.0 — real patch tags are forbidden (the git SHA is the patch identity; FR-7 / §Q2)" >&2
    exit 2
fi
VERSION="${TAG#v}" # X.Y.0

if ! grep -qF "## [$VERSION]" CHANGELOG.md; then
    echo "error: CHANGELOG.md has no section '## [$VERSION]' for $TAG — every release must be changelog-recorded" >&2
    exit 2
fi

echo "tag: $TAG (canonical vMAJOR.MINOR.0 — FR-7)"
echo "changelog: '## [$VERSION]' present"

# --- signature REPORT (never a gate — CI holds no private key, Q6) ---------
KEY="docs/release-signing-key.asc"
PRINCIPAL="$(awk '/ssh-ed25519/ {print $1; exit}' "$KEY" 2>/dev/null || true)"
if [ -n "$PRINCIPAL" ] && git rev-parse -q --verify "refs/tags/$TAG" >/dev/null 2>&1; then
    if git -c gpg.format=ssh -c gpg.ssh.allowedSignersFile="$KEY" \
        tag -v "$TAG" >/dev/null 2>&1; then
        echo "TAG-SIGNATURE: VERIFIED by $PRINCIPAL (allowedSignersFile=$KEY)"
    else
        echo "TAG-SIGNATURE: UNVERIFIED locally (GitHub's Verified badge is the primary signal; CI holds no private key — Q6)"
    fi
else
    echo "TAG-SIGNATURE: not checked (tag $TAG not present in this checkout)"
fi
exit 0