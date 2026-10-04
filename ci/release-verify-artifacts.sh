#!/usr/bin/env bash
# PLAN-RELEASE-1 S9 — artifact well-formedness gate (FR-18 gate step 7, §6.4).
#
# The LAST release gate. Runs on a v* tag push in release.yml's `artifacts`
# job. From the tagged tree and the DRAFT GitHub release it verifies:
#   1. the release for the tag EXISTS and is still a DRAFT — a release gate
#      must never publish (FR-18), and a premature publish is a red flag;
#   2. the draft carries all three §6.4 attachments (tarball / .sha256 / .sig);
#   3. the attached tarball is BYTE-IDENTICAL to a `git archive` recomputed
#      from the tag — a cut that attached the wrong content fails here;
#   4. the commited checksum verifies the attached tarball (sha256);
#   5. the attached SSH signature decodes and validates the tarball against
#      the COMMITTED public key (docs/release-signing-key.asc) — the exact
#      `ssh-keygen -Y verify` snippet SECURITY.md/RELEASES.md give consumers.
#
# CI holds NO private signing key (Q6): this is well-formedness only. The
# signature was produced once, on the maintainer's machine, by
# ci/cut-release.sh.
#
# Usage: ci/release-verify-artifacts.sh <owner/repo> vMAJOR.MINOR.0
#
# Requires: gh (authenticated), jq, ssh-keygen. In CI, GH_TOKEN/GITHUB_TOKEN
# provides repo-scoped read access to the draft.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

REPO="${1:-}"
TAG="${2:-}"
if [ -z "$REPO" ] || [ -z "$TAG" ]; then
    echo "usage: ci/release-verify-artifacts.sh <owner/repo> vMAJOR.MINOR.0" >&2
    exit 2
fi
if ! [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.0$ ]]; then
    echo "error: tag '$TAG' is not canonical vMAJOR.MINOR.0 (FR-7 / §Q2)" >&2
    exit 2
fi

VERSION="${TAG#v}"            # X.Y.0
BASE="tyu-$VERSION"           # §6.4 artifact stem: tyu-X.Y.0
KEY="docs/release-signing-key.asc"
PRINCIPAL="$(awk '/ssh-ed25519/ {print $1; exit}' "$KEY")"
[ -n "$PRINCIPAL" ] || { echo "error: no signer principal found in $KEY" >&2; exit 1; }

die() { echo "error: $*" >&2; exit 1; }
say() { echo "  $*"; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

command -v gh >/dev/null 2>&1 || die "gh is required (draft release reading)"
command -v jq >/dev/null 2>&1 || die "jq is required"
command -v ssh-keygen >/dev/null 2>&1 || die "ssh-keygen is required"

# --- 1. recompute the tarball from the tag ----------------------------------
if ! git rev-parse -q --verify "refs/tags/$TAG" >/dev/null 2>&1; then
    die "tag $TAG not present in this checkout (the artifacts gate runs from the tagged SHA)"
fi
git archive --format=tar.gz -o "$WORK/recomputed.tar.gz" "$TAG"
[ -s "$WORK/recomputed.tar.gz" ] || die "git archive of $TAG produced nothing"

# --- 2/3. read the draft + confirm all three §6.4 attachments ---------------
# NOTE: `gh api repos/.../releases/tags/<tag>` does NOT return DRAFT releases
# (the per-tag endpoint serves published releases only) — a draft must be read
# through `gh release view`/`gh release download`, which handle drafts for an
# authenticated caller with push access (S9 audit finding).
RELEASE_JSON="$WORK/release.json"
if ! gh release view "$TAG" --repo "$REPO" --json isDraft,assets > "$RELEASE_JSON" 2>/dev/null; then
    die "cannot read release $TAG from $REPO — does the draft exist? is gh authenticated (GH_TOKEN with push access sees drafts)?"
fi
DRAFT="$(jq -r '.isDraft' "$RELEASE_JSON" 2>/dev/null)"
[ "$DRAFT" = "true" ] || die "release $TAG is not a DRAFT (isDraft=$DRAFT) — a release gate must never publish (FR-18)"

for asset in "$BASE.tar.gz" "$BASE.tar.gz.sha256" "$BASE.tar.gz.sig"; do
    jq -e --arg a "$asset" '.assets[] | select(.name == $a) | .name' "$RELEASE_JSON" >/dev/null 2>&1 \
        || die "draft $TAG lacks the §6.4 attachment '$asset'"
    gh release download "$TAG" --repo "$REPO" --pattern "$asset" --dir "$WORK" --clobber \
        || die "cannot download attachment '$asset'"
    [ -s "$WORK/$asset" ] || die "attachment '$asset' downloaded empty"
    say "attachment: $asset ($(wc -c < "$WORK/$asset") bytes)"
done

# --- 4. recomputed == attached + attached checksum verifies ------------------
cmp -s "$WORK/recomputed.tar.gz" "$WORK/$BASE.tar.gz" \
    || die "attached tarball differs from 'git archive $TAG' — a cut attached the wrong content"
say "tarball: attached $BASE.tar.gz is byte-identical to git archive of $TAG"

ATTACHED_HASH="$(awk '{print $1}' "$WORK/$BASE.tar.gz.sha256")"
if ! [[ "$ATTACHED_HASH" =~ ^[0-9a-f]{64}$ ]]; then
    die "attached $BASE.tar.gz.sha256 does not contain a 64-hex SHA-256 (malformed checksum)"
fi
COMPUTED_HASH="$(sha256sum "$WORK/$BASE.tar.gz" | awk '{print $1}')"
[ "$ATTACHED_HASH" = "$COMPUTED_HASH" ] \
    || die "attached checksum ($ATTACHED_HASH) does not match the attached tarball ($COMPUTED_HASH)"
say "checksum: attached SHA-256 verifies the attached tarball ($COMPUTED_HASH)"

# --- 5. SSH signature decodes against the committed public key --------------
ssh-keygen -Y verify \
    -f "$KEY" \
    -I "$PRINCIPAL" \
    -n git \
    -s "$WORK/$BASE.tar.gz.sig" \
    < "$WORK/$BASE.tar.gz" >/dev/null 2>&1 \
    || die "attached signature does not verify against $KEY (principal $PRINCIPAL, namespace git)"
say "signature: detached SSH signature validates ($KEY / $PRINCIPAL / namespace git)"

echo ""
echo "ARTIFACT-VERIFY: PASS — draft $TAG carries canonical, checksum-consistent, signature-valid $BASE artifacts (§6.4)"
exit 0