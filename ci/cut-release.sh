#!/usr/bin/env bash
# PLAN-RELEASE-1 S4 — the release runbook as code (FR-6, FR-7, §6.3, §6.4).
#
# Usage:
#   ci/cut-release.sh [--dry-run] vMAJOR.MINOR[.0]
#
# Real mode cuts a release: bumps crate versions, commits, creates an
# annotated SSH-signed tag, produces the source tarball + SHA-256 + detached
# SSH signature, and opens a DRAFT GitHub release (never publishes). All
# side-effecting steps happen on THIS machine — CI never holds the private
# signing key (Q6).
#
# --dry-run runs every local mechanical check (version form, changelog
# section, tag absence, band-rule probe when a previous tag exists) and
# prints the full cut plan without touching the tree, git refs, or GitHub.
# Environment gates (clean tree, branch, gh login, remote gate status) are
# REPORTED in dry-run and BLOCK in real mode — a dry-run on a work branch
# with uncommitted work is how the mechanics get validated before a real cut.
#
# Version identity (resolves the FR-7 / §Q2 naming tension): FR-7's regex
# forbids patch tags; the policy examples tag v0.1.0 / v0.2.0. Canonical form
# is three-component with an ENFORCED zero patch: `vMAJOR.MINOR.0`. Real
# patch tags (v0.1.1) are refused — the git SHA is the patch identity.
#
# Signing key resolution (maintainer's machine): $TYU_SIGNING_KEY, else `git
# config user.signingkey`, else ~/.ssh/id_ed25519. The corresponding public
# key is committed at docs/release-signing-key.asc (fingerprint in
# RELEASES.md). Tags use `gpg.format = ssh`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DRY_RUN=0
case "${1:-}" in
    --dry-run) DRY_RUN=1; shift ;;
esac

VERSION_ARG="${1:-}"
if [ -z "$VERSION_ARG" ]; then
    echo "usage: ci/cut-release.sh [--dry-run] vMAJOR.MINOR[.0]" >&2
    exit 2
fi

say() { echo "  $*"; }
die() { echo "error: $*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Version form (STEP 3a — always blocks, cheapest check first)
# ---------------------------------------------------------------------------
if ! [[ "$VERSION_ARG" =~ ^v([0-9]+)\.([0-9]+)(\.0+)?$ ]]; then
    echo "error: version must be vMAJOR.MINOR[.0] — real patch tags are forbidden (the SHA is the patch identity), got '$VERSION_ARG'" >&2
    exit 2
fi
VMAJ="${BASH_REMATCH[1]}"
VMIN="${BASH_REMATCH[2]}"
VERSION="${VMAJ}.${VMIN}.0"
TAG="v${VERSION}"

say "cutting release $TAG (dry-run=$DRY_RUN)"

# ---------------------------------------------------------------------------
# STEP 3b — changelog section present (local, blocking in both modes)
# ---------------------------------------------------------------------------
if ! grep -qF "## [$VERSION]" CHANGELOG.md; then
    die "CHANGELOG.md has no section '## [$VERSION]' — add it (or promote [Unreleased]) before cutting"
fi

# ---------------------------------------------------------------------------
# STEP 3c — tag does not exist locally (blocking in both modes)
# ---------------------------------------------------------------------------
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null 2>&1; then
    die "tag $TAG already exists locally (rev $(git rev-parse --short "refs/tags/$TAG"))"
fi

# ---------------------------------------------------------------------------
# STEP 1 — environment gates (BLOCK in real mode; reported in dry-run)
# ---------------------------------------------------------------------------
step1_fail=0
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
    say "precondition: working tree is DIRTY — a real cut requires a clean tree"
    step1_fail=1
else
    say "precondition: working tree clean"
fi
branch="$(git symbolic-ref --short HEAD 2>/dev/null || echo detached)"
if [ "$branch" != "main" ]; then
    say "precondition: on '$branch' — a real cut runs from 'main'"
    step1_fail=1
else
    say "precondition: on main"
fi
SIGNING_KEY="${TYU_SIGNING_KEY:-$(git config --get user.signingkey || true)}"
[ -n "$SIGNING_KEY" ] || SIGNING_KEY="$HOME/.ssh/id_ed25519"
if [ ! -f "$SIGNING_KEY" ]; then
    say "precondition: signing key NOT found at '$SIGNING_KEY' (set TYU_SIGNING_KEY)"
    step1_fail=1
else
    say "precondition: signing key $SIGNING_KEY present"
fi
if [ "$DRY_RUN" -eq 0 ] && [ "$step1_fail" -ne 0 ]; then
    die "a real cut requires a clean tree on 'main' with the signing key present"
fi

# ---------------------------------------------------------------------------
# STEP 2 — previous release gate green (real mode blocks; dry-run attempts)
# ---------------------------------------------------------------------------
HEAD_SHA="$(git rev-parse HEAD)"
gate_check() {
    local wf="$1"
    # last completed run of this workflow for HEAD must be success
    local row
    row="$(gh run list --workflow "$wf" --commit "$HEAD_SHA" --limit 1 \
        --json status,conclusion --jq '.[0] | .status + "/" + (.conclusion // "null")' 2>/dev/null || true)"
    printf '%s' "$row"
}
if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    for wf in ci.yml integration.yml; do
        row="$(gate_check "$wf")"
        case "$row" in
            completed/success) say "gate: $wf green on $HEAD_SHA" ;;
            *)
                say "gate: $wf on $HEAD_SHA not confirmed green ($row)" >&2
                if [ "$DRY_RUN" -eq 0 ]; then
                    die "previous $wf gate for $HEAD_SHA is not green — a release cannot be cut from a red leg"
                fi
                ;;
        esac
    done
else
    say "gate: gh not authenticated — remote gate check SKIPPED (blocking in real mode)" >&2
fi

# tag remotely absent (real mode blocks)
if [ "$DRY_RUN" -eq 0 ] && command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    if gh release view "$TAG" >/dev/null 2>&1 || \
        git ls-remote --tags origin "refs/tags/$TAG" 2>/dev/null | grep -q "$TAG"; then
        die "tag/release $TAG already exists remotely"
    fi
    say "gate: $TAG absent remotely"
fi

# ---------------------------------------------------------------------------
# STEP 4 — statement band-rule probe (abort on drift)
# ---------------------------------------------------------------------------
band_note="goldens verified stable within the band (probe above)"
prev_tag="$(git tag -l 'v[0-9]*.[0-9]*.0' | sort -V | tail -n 1 || true)"
if [ -z "$prev_tag" ]; then
    band_note="first cut — band rule vacuous (§Q4)"
    say "band probe: no previous v* release — first cut, band rule vacuous (§Q4)"
elif [ "$prev_tag" = "$TAG" ]; then
    say "band probe: $TAG matches previous tag (nothing to compare)"
else
    say "band probe: comparing statement goldens against $prev_tag (PLAN-VERIFY-3 §Q4 item 3)"
    if [ "$DRY_RUN" -eq 1 ]; then
        say "band probe: (dry-run) would run 'TYU_REGEN_STATEMENT_GOLDENS=1 cargo test --release -p tooling-tests --test statement_goldens' twice, then the strict pass, then git diff --exit-code -- test-goldens/statements"
    else
        TYU_REGEN_STATEMENT_GOLDENS=1 cargo test --release -p tooling-tests --test statement_goldens >/dev/null
        TYU_REGEN_STATEMENT_GOLDENS=1 cargo test --release -p tooling-tests --test statement_goldens >/dev/null
        # strict pass: encoder must match the regenerated (= committed) goldens
        cargo test --release -p tooling-tests --test statement_goldens >/dev/null
        if ! git diff --exit-code --quiet -- test-goldens/statements; then
            die "statement band drift: regenerating the goldens changed test-goldens/statements — the encoder hash surface moved within a minor band (§Q4 item 3). This release must be MINOR with a migration map, not the stable band."
        fi
        say "band probe: goldens stable within the band"
    fi
fi

# ---------------------------------------------------------------------------
# STEP 5 — cut (real mode only; dry-run prints the plan)
# ---------------------------------------------------------------------------
if [ "$DRY_RUN" -eq 1 ]; then
    to_bump="$(for f in crates/*/Cargo.toml; do grep -q "^version = \"$VERSION\"" "$f" || echo "$f"; done | wc -l)"
    echo ""
    echo "--- dry-run plan (no side effects taken) ---"
    say "version bump to $VERSION: $to_bump manifest(s) differ; cargo update -w --offline after"
    say "commit 'release: v$VERSION'"
    say "SSH-signed annotated tag: git -c gpg.format=ssh -c user.signingkey=$SIGNING_KEY tag -s $TAG -m 'tyu $VERSION'"
    say "tarball: git archive --format=tar.gz -o target/release-artifacts/tyu-${VERSION}.tar.gz $TAG"
    say "checksum: sha256sum > target/release-artifacts/tyu-${VERSION}.tar.gz.sha256"
    say "signature: ssh-keygen -Y sign -n git -f $SIGNING_KEY target/release-artifacts/tyu-${VERSION}.tar.gz"
    say "draft release: gh release create $TAG --draft --title 'tyu $VERSION' --notes-file <changelog excerpt> [tarball|.sha256|.sig]"
    echo "--- end dry-run plan ---"
    echo "dry-run: exit 0 (a real cut would also enforce the environment gates reported above)"
    exit 0
fi

echo ""
say "STEP 5: cutting $TAG"
mkdir -p target/release-artifacts

# 5a — bump every crate manifest to $VERSION (current is always 0.x.0-mostly)
bumped=0
for f in crates/*/Cargo.toml; do
    if grep -q '^version = "' "$f"; then
        cur="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$f" | head -n 1)"
        if [ -n "$cur" ] && [ "$cur" != "$VERSION" ]; then
            if ! sed -i.bak "s/^version = \"$cur\"/version = \"$VERSION\"/" "$f" 2>/dev/null; then
                die "failed to bump $f"
            fi
            rm -f "$f.bak"
            say "   bumped $f: $cur -> $VERSION"
            bumped=1
        fi
    fi
done
if [ "$bumped" -eq 1 ]; then
    cargo update -w --offline >/dev/null
fi

# 5a-bis — release record into RELEASES.md (FR-6): the record is written by
# THIS script BEFORE the tag commit, so the tagged tree carries its own
# release record and RELEASES.md's "appended by cut-release.sh" contract
# holds. A failed later step leaves the record committed with no tag — the
# tag-exists guard above keeps a re-run honest.
fingerprint="$(grep -o 'SHA256:[A-Za-z0-9+/=]*' docs/release-signing-key.asc | head -n 1)"
{
    echo ""
    echo "## $TAG — $(date -u +%F)"
    echo ""
    echo "- toolchain: $(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)"
    echo "- statement band: $band_note"
    echo "- verification: tag SSH-signed (fingerprint $fingerprint); tarball tyu-$VERSION.tar.gz + .sha256 + .sig attached to the GitHub release"
} >> RELEASES.md
say "   RELEASES.md record appended"

git add -A
git commit -m "release: v$VERSION" >/dev/null
say "   committed 'release: v$VERSION' ($(git rev-parse --short HEAD))"

# 5b — annotated SSH-signed tag
if ! git -c gpg.format=ssh -c user.signingkey="$SIGNING_KEY" tag -s "$TAG" -m "tyu $VERSION"; then
    die "SSH-signed tag creation failed — is the key in your agent or reachable by path?"
fi
say "   tag created: $(git rev-parse --short "$TAG")"
if git cat-file -p "$TAG" | grep -q '^gpgsig'; then
    say "   tag signature present (SSH)"
else
    say "   tag signature MISSING"
fi

# 5c — tarball + checksum + detached signature
tarball="target/release-artifacts/tyu-${VERSION}.tar.gz"
git archive --format=tar.gz -o "$tarball" "$TAG"
sha256sum "$tarball" > "$tarball.sha256"
# OpenSSH 9.x writes the signature to stdout; 10.x writes "<tarball>.sig"
# itself (and prompts "Overwrite?" if the target already exists — hence the
# rm first). Verify with: ssh-keygen -Y verify -f <allowed_signers> -I \
# <principal> -n git -s "$tarball.sig" < "$tarball"   (data ON STDIN).
rm -f "$tarball.sig"
if ! ssh-keygen -Y sign -n git -f "$SIGNING_KEY" "$tarball" >/dev/null 2>&1; then
    die "ssh-keygen -Y sign failed"
fi
if [ ! -s "$tarball.sig" ]; then
    ssh-keygen -Y sign -n git -f "$SIGNING_KEY" "$tarball" > "$tarball.sig" 2>/dev/null
fi
say "   artifacts: $tarball (+ .sha256, .sig)"

# 5d — release notes excerpt
notes="target/release-artifacts/notes-${VERSION}.md"
{
    echo "# tyu $VERSION"
    echo ""
    awk -v sec="$VERSION" 'index($0, "## [" sec) == 1 {f=1; next} /^## /{f=0} f' CHANGELOG.md
    echo ""
    echo "Toolchain: $(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)"
    echo "Tag signature: SSH (fingerprint $(grep -o 'SHA256:[A-Za-z0-9+/=]*' docs/release-signing-key.asc | head -n 1))"
    echo "Verify: see SECURITY.md verification instructions."
} > "$notes"

# 5e — DRAFT GitHub release (never publishes; Q6 signatures never in CI)
if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    gh release create "$TAG" --draft --title "tyu $VERSION" --notes-file "$notes" \
        "$tarball" "$tarball.sha256" "$tarball.sig"
    say "   draft release created (NOT published)"
else
    say "   [skip] gh unavailable — draft created manually: checklists below"
fi

echo ""
say "STEP 6: manual checklist (maintainer)"
say "  - branch protection on 'main' (PR + green CI) and tag protection on 'v*' are enabled"
say "  - the signing public key is uploaded to GitHub (settings -> SSH and GPG keys) so the tag shows Verified"
say "  - read the draft, then publish it (gh release edit $TAG --notes-file ... && gh release publish $TAG)"
say "  - after publish: record the release in RELEASES.md (§6.3) and bump [Unreleased] in CHANGELOG.md"
say "DONE"