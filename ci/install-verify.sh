#!/usr/bin/env bash
# PLAN-RELEASE-1 S6 / FR-9 — pristine-container install verification.
#
# Runs INSIDE an ubuntu:24.04 container with the repository checked out
# read-only at /repo — the EXACT tree under test (the pull_request path
# filter means a PR that touches the install path proves its own tree here).
#
# What it does:
#   1. pristine base: git, curl, build-essential (nothing else preinstalled)
#   2. executes ci/install-verify-commands.txt — the quickstart, byte-for-byte
#      identical to the SETUP.md fence (a ci-lint gate enforces that); the
#      verifier sets TYU_REPO="file:///repo" so `bash ci/install.sh` clones
#      exactly the tested commit, provisions rustup (SHA-256-verified) and
#      the pinned nightly, builds the host binaries, and installs them.
#   3. asserts the health surface (tyu doctor; a forward contract until the
#      S7 slice — detected and skipped transparently here, HARD in S7)
#   4. asserts the red smoke (ci/smoke/bad.mod must trap CONTRACT_FAIL = 20)
#
# CI YAML stays thin: the workflow just mounts the checkout and runs this
# script (shellcheck-covered; see ci-lint.sh).
set -euo pipefail

CHECKOUT="$(cd "${VERIFY_CHECKOUT:-/repo}" && pwd)"
PREFIX="${TYU_PREFIX:-$HOME/.tyu}"
RUN_LOG="${RUN_LOG:-/tmp/install-verify.log}"
WORKDIR="$(mktemp -d)"

say() { echo "== install-verify: $*"; }
fail() { echo "install-verify: FAIL — $*" >&2; exit 1; }

[ -d "$CHECKOUT/.git" ] || fail "$CHECKOUT is not a git checkout (mount the repo at /repo)"

# --- pristine baseline -----------------------------------------------------
apt-get update -qq
# build-essential + git + curl + fasm + binutils: the hosted smoke compiles
# x86_64 modules with fasm and links with ld (plan S6 finding — the smoke
# cannot assemble without fasm).
apt-get install -y -qq git curl build-essential fasm binutils
# the checkout is owned by the host user (read-only mount); git refuses
# "dubious ownership" otherwise. The client-side rev-parse checks the worktree
# root; the file:// upload-pack server checks the .git dir. Both are the
# documented, expected fixes for a ro-mount in a container (plan S6 finding).
git config --global --add safe.directory "$CHECKOUT"
git config --global --add safe.directory "$CHECKOUT/.git"

SHA="$(git -C "$CHECKOUT" rev-parse HEAD)"
say "tree under test: $SHA"

# --- the verified quickstart (runs as one shell, like a real user) --------
# TYU_REPO must be a repo whose refs/heads/main IS the tested commit: the
# mounted Actions checkout is a detached HEAD with no refs/heads (a branch
# clone from it fails with "Remote branch main not found" — audit finding,
# S6), and even where a branch ref existed, a branch clone would fetch that
# branch's tip, not the tree under test. A writable local clone of the
# checkout's HEAD, with its HEAD named 'main', makes the installer's default
# ref resolution (newest v* tag -> else main) clone EXACTLY the tested commit.
SRC_REPO="$WORKDIR/src"
git clone --quiet "$CHECKOUT" "$SRC_REPO"
git -C "$SRC_REPO" branch --force main HEAD
set +e
(
    cd "$CHECKOUT"
    export TYU_REPO="$SRC_REPO"
    export TYU_PREFIX="$PREFIX"
    bash -e "$CHECKOUT/ci/install-verify-commands.txt"
) | tee "$RUN_LOG"
quickstart_rc="${PIPESTATUS[0]}"
set -e
[ "$quickstart_rc" -eq 0 ] || fail "the quickstart command failed (rc=$quickstart_rc) — see the log above"
say "quickstart executed end-to-end (installer -> build -> green smoke)"

# --- health surface --------------------------------------------------------
export PATH="$PREFIX/bin:$PATH"
# doctor lands with the S7 slice; until then it is a forward contract. The
# verifier feature-detects and skips transparently; S7 tightens this to a
# hard requirement (a doctor that reports failures fails the job).
doctor_json="$PREFIX/doctor.json"
doctor_ok=0
if "$PREFIX/bin/tyu" doctor --tier=hosted --format=json > "$doctor_json" 2>/dev/null; then
    say "doctor: present — asserting hosted tier healthy"
    doctor_ok=1
    grep -q '"fail": 0' "$doctor_json" \
        || { cat "$doctor_json" >&2; fail "doctor reports failures on the installed toolchain"; }
else
    say "doctor: not on this tree yet (arrives with the S7 slice) — skipped transparently"
    rm -f "$doctor_json"
fi

# --- red smoke: must trap CONTRACT_FAIL (code 20) --------------------------
# run from the tested checkout (the quickstart subshell already left CWD=/)
bad_out="$WORKDIR/bad"
if ! ( cd "$CHECKOUT" && "$PREFIX/bin/tyu" build ci/smoke/bad.mod --target=x86_64-unknown-linux-gnu \
        --out-dir="$bad_out" ) >/dev/null 2>&1; then
    fail "ci/smoke/bad.mod did not build"
fi
set +e
"$bad_out/image.elf"
bad_rc=$?
set -e
if [ "$bad_rc" -ne 20 ]; then
    fail "ci/smoke/bad.mod image exit $bad_rc (want 20 / CONTRACT_FAIL)"
fi
say "red smoke: CONTRACT_FAIL (exit 20) exactly as pinned"

# --- job summary (install log tail + doctor JSON) ---------------------------
# The container writes into VERIFY_OUT (a temp dir the workflow mounts, since
# a container cannot reach the runner's step-summary path). The workflow's
# follow-up step renders it into the GitHub step summary.
verify_out="${VERIFY_OUT:-}"
if [ -n "$verify_out" ]; then
    {
        echo "### Install verification"
        echo ""
        echo "Tree under test: \`$SHA\`"
        echo "Quickstart commands: \`ci/install-verify-commands.txt\` (matches the SETUP.md fence; ci-lint-enforced)"
        echo ""
        echo "- green smoke: exit 0"
        echo "- red smoke: exit 20 (\`CONTRACT_FAIL\`)"
        if [ "$doctor_ok" -eq 1 ]; then
            echo "- doctor: hosted tier healthy"
        fi
        echo ""
        echo "Install log (tail):"
        echo "\`\`\`"
        tail -n 15 "$RUN_LOG" 2>/dev/null || true
        echo "\`\`\`"
        if [ -f "$doctor_json" ]; then
            echo ""
            echo "doctor JSON (\`tyu.doctor/1\`):"
            echo "\`\`\`json"
            cat "$doctor_json"
            echo "\`\`\`"
        fi
    } > "$verify_out/summary.md"
fi

say "INSTALL-VERIFY: PASS"
rm -rf "$WORKDIR"
exit 0