#!/usr/bin/env bash
# PLAN-RELEASE-1 S5 — the blessed source installer (FR-8, Q4, Q10).
#
# Usage:
#   ci/install.sh [--tag vMAJOR.MINOR[.0]] [--branch NAME] [--prefix DIR] [--dry-run]
#
# Installs the tyu toolchain by building it FROM SOURCE into a user prefix.
# There are no prebuilt binaries (Q2/non-goal): this is the blessed path.
#
# Stages (each fail-closed with a named remediation — Q10):
#   ensure-git    git present
#   ensure-cc     a C linker/compiler present (Rust links against one)
#   ensure-rustup rustup/cargo present, user-local (official installer,
#                 downloaded and SHA-256-VERIFIED before execution)
#   resolve-ref   newest v* tag on the origin (or --tag/--branch); main
#                 fallback is visible, never silent
#   clone         durable checkout at $prefix/src/tyu (repo-anchored: the
#                 compiled-in workspace root must be a real path, so the
#                 checkout is NOT deleted after the build); re-runs shallow-
#                 fetch + force-checkout in place (idempotent, cache reused)
#   build         cargo build --release of the host binary set (the pinned
#                 nightly provisions itself from rust-toolchain.toml)
#   install-bin   atomic move of the binaries into $prefix/bin
#   record        $prefix/INSTALL_RECORD written LAST
#   advise        PATH export, repo-anchored usage contract, `tyu doctor`
#
# Properties: zero sudo (the rustup installer is the single piped script, run
# user-local), idempotent re-run (fresh temp clone + atomic move every time),
# no partial state on failure, PATH-collision warning at the end. SEE ALSO the
# verify-first rustup two-step under --help.
set -euo pipefail

PREFIX="${TYU_PREFIX:-$HOME/.tyu}"
REPO="${TYU_REPO:-https://github.com/lexbity/tyu.git}"
TAG=""
BRANCH=""
DRY_RUN=0
CHECKOUT_SHA=""

say() { echo "  $*"; }
die() { echo "error: $*" >&2; exit 1; }

usage() {
    cat <<EOF
usage: ci/install.sh [--tag vMAJOR.MINOR[.0]] [--branch NAME] [--prefix DIR] [--dry-run]

  --tag vX.Y[.0]    install that release tag (must exist on the origin)
  --branch NAME     install a branch of the origin (default source of truth)
  --prefix DIR      install prefix (default \$HOME/.tyu; env TYU_PREFIX)
  --dry-run         print the plan without touching anything

rustup bootstrap, verify-first two-step (recommended; install.sh runs the
same two steps itself when cargo is absent):
  curl -fsSLO https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init
  curl -fsS https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init.sha256 | sha256sum -c -
  chmod +x ./rustup-init && ./rustup-init -y --default-toolchain none --no-modify-path

Env: TYU_REPO (origin URL), TYU_PREFIX. Zero sudo anywhere.
EOF
}

# --- argument parsing -------------------------------------------------------
while [ "$#" -gt 0 ]; do
    case "$1" in
        --tag) TAG="${2:-}"; shift 2 ;;
        --branch) BRANCH="${2:-}"; shift 2 ;;
        --prefix) PREFIX="${2:-}"; shift 2 ;;
        --dry-run) DRY_RUN=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) echo "error: unknown argument '$1'" >&2; usage >&2; exit 2 ;;
    esac
done

if [ -n "$TAG" ] && [ -n "$BRANCH" ]; then
    die "--tag and --branch are mutually exclusive"
fi

# HOST_BIN_SET mirrors ci.yml/FR-8 exactly.
HOST_BIN_SET=(langc tyu lmod-pack lmod-sign lmod-encrypt)

# ===========================================================================
# stage: ensure-git
# ===========================================================================
if [ "$DRY_RUN" -eq 1 ]; then
    say "ensure-git   : git required (apt: git | dnf: git | pacman: git | brew: git)"
else
    if ! command -v git >/dev/null 2>&1; then
        die "git not found — install it (apt: sudo apt-get install -y git)"
    fi
fi

# ===========================================================================
# stage: ensure-cc
# ===========================================================================
if [ "$DRY_RUN" -eq 1 ]; then
    say "ensure-cc    : a C linker/compiler required (apt: build-essential | dnf: gcc | pacman: base-devel | brew: gcc)"
else
    if ! command -v cc >/dev/null 2>&1 && ! command -v gcc >/dev/null 2>&1 \
        && ! command -v clang >/dev/null 2>&1; then
        die "no C linker/compiler found (cc/gcc/clang) — Rust links with one (apt: sudo apt-get install -y build-essential)"
    fi
fi

# ===========================================================================
# stage: ensure-rustup (user-local; verified download before execution)
# ===========================================================================
if [ "$DRY_RUN" -eq 1 ]; then
    say "ensure-rustup: rustup/cargo; if absent, install user-local (official rustup-init.sh, SHA-256-verified)"
else
    if command -v cargo >/dev/null 2>&1 && command -v rustup >/dev/null 2>&1; then
        say "ensure-rustup: rustup present ($(rustc --version 2>/dev/null | head -n 1 || echo unknown))"
    else
        say "ensure-rustup: installing rustup user-local (no sudo)"
        rustup_host="${RUSTUP_HOST:-x86_64-unknown-linux-gnu}"
        base="https://static.rust-lang.org/rustup/dist/${rustup_host}"
        rustup_tmp="$(mktemp -d)"
        curl -fsSL "${base}/rustup-init" -o "${rustup_tmp}/rustup-init"
        expected="$(curl -fsSL "${base}/rustup-init.sha256" | awk '{print $1}')"
        actual="$(sha256sum "${rustup_tmp}/rustup-init" | awk '{print $1}')"
        if [ "$expected" != "$actual" ]; then
            rm -rf "$rustup_tmp"
            die "rustup-init checksum verification FAILED (expected $expected, got $actual) — refusing to execute"
        fi
        chmod +x "${rustup_tmp}/rustup-init"
        "${rustup_tmp}/rustup-init" -y --default-toolchain none --no-modify-path
        rm -rf "$rustup_tmp"
        # rustup-init placed the binaries under HOME/.cargo/bin; path them now
        # without editing any shell rc (the advise stage prints the export).
        PATH="$HOME/.cargo/bin:$PATH"
        if ! command -v cargo >/dev/null 2>&1; then
            die "rustup installed but cargo not on PATH — restart the shell or export PATH=\$HOME/.cargo/bin:\$PATH and re-run"
        fi
    fi
fi

# ===========================================================================
# stage: resolve-ref
# ===========================================================================
# REF_IS_DEFAULT marks the no-flag default resolution. The default clones the
# origin's HEAD rather than a hardcoded "main": on a normal origin HEAD is
# what a bare `git clone` gets (main), and on the CI install-verify fixture
# (a detached checkout of the exact tested commit) HEAD IS that commit — so
# "install the checked tree" needs no special ref plumbing.
REF="$BRANCH"
REF_IS_DEFAULT=0
if [ -n "$TAG" ]; then
    REF="$TAG"
    # a requested tag must actually exist on the origin
    if [ "$DRY_RUN" -eq 0 ]; then
        if ! git ls-remote --tags --refs "$REPO" "refs/tags/$TAG" 2>/dev/null | grep -q "$TAG"; then
            die "tag '$TAG' not found on $REPO"
        fi
    fi
elif [ -z "$BRANCH" ]; then
    newest_tag="$(git ls-remote --tags --refs "$REPO" 'v*' 2>/dev/null \
        | sed 's#.*refs/tags/##' | sort -V | tail -n 1 || true)"
    if [ -n "$newest_tag" ]; then
        REF="$newest_tag"
        say "resolve-ref  : newest release tag on the origin: $REF"
    else
        REF="HEAD"
        REF_IS_DEFAULT=1
        say "resolve-ref  : no v* tags on the origin — defaulting to the origin HEAD (main)"
    fi
fi

# a bare 40-hex SHA ref (e.g. CI's "--branch <sha>") needs a clone + checkout,
# not a --branch fetch (git clone --branch takes ref names).
if [[ "$REF" =~ ^[0-9a-f]{40}$ ]]; then
    say "resolve-ref  : ref '$REF' is a raw commit SHA"
fi

# ===========================================================================
# stage: clone (durable checkout at $prefix/src/tyu)
# ===========================================================================
# The toolchain is repo-anchored: the workspace root (runtime/, ports/,
# platforms/, sysroot/) is resolved from the compiled-in workspace root. The
# build therefore happens IN a durable checkout at $prefix/src/tyu (the Q4
# destination) so that baked root is a real, permanent path — a temp clone
# that is deleted after the build would leave a toolchain that points into
# the void (verified S5: installed tyu failed to find runtime.asm). Re-runs
# update the checkout in place (shallow fetch + forced checkout), so the
# build cache is reused and idempotency holds; $prefix/bin is still replaced
# atomically and INSTALL_RECORD written last (Q10).
SRC_CHECKOUT="$PREFIX/src/tyu"
if [ "$DRY_RUN" -eq 1 ]; then
    say "clone        : $REF from $REPO -> $SRC_CHECKOUT (durable repo anchor)"
else
    say "clone        : $REF from $REPO -> $SRC_CHECKOUT"
    if [ -d "$SRC_CHECKOUT/.git" ]; then
        # idempotent in-place update: shallow-fetch the ref and force it out
        if [ "$REF_IS_DEFAULT" -eq 1 ]; then
            # default ref: fetch the origin HEAD, then reset to it.
            if ! git -C "$SRC_CHECKOUT" fetch --quiet --depth 1 --force origin \
                || ! git -C "$SRC_CHECKOUT" reset --hard --quiet FETCH_HEAD; then
                die "update of $SRC_CHECKOUT to the origin HEAD failed"
            fi
        else
            if ! git -C "$SRC_CHECKOUT" fetch --quiet --depth 1 origin "$REF" --force \
                && ! git -C "$SRC_CHECKOUT" fetch --quiet --depth 1 origin "refs/tags/$REF" --force; then
                die "update of $SRC_CHECKOUT to $REF failed"
            fi
            git -C "$SRC_CHECKOUT" checkout --force --quiet "$REF" 2>/dev/null \
                || git -C "$SRC_CHECKOUT" checkout --force --quiet "refs/tags/$REF"
        fi
    elif [[ "$REF" =~ ^[0-9a-f]{40}$ ]]; then
        # raw SHA: clone the origin's default branch, then detach at the SHA
        # (works even when the origin is a shallow clone whose HEAD is the SHA)
        mkdir -p "$PREFIX/src"
        if ! git clone --quiet "$REPO" "$SRC_CHECKOUT"; then
            rm -rf "$SRC_CHECKOUT"
            die "clone of $REPO failed — check the URL and network"
        fi
        if ! git -C "$SRC_CHECKOUT" checkout --quiet --detach "$REF"; then
            rm -rf "$SRC_CHECKOUT"
            die "cannot check out '$REF' from a clone of $REPO (is it reachable?)"
        fi
    elif [ "$REF_IS_DEFAULT" -eq 1 ]; then
        # default: clone the origin HEAD (main on a normal origin; the exact
        # checked commit on the CI install-verify fixture)
        mkdir -p "$PREFIX/src"
        if ! git clone --quiet --depth 1 --single-branch "$REPO" "$SRC_CHECKOUT"; then
            rm -rf "$SRC_CHECKOUT"
            die "clone of $REPO failed — check the URL and network"
        fi
    else
        # explicit branch/tag: shallow single-branch at that ref
        mkdir -p "$PREFIX/src"
        if ! git clone --quiet --depth 1 --branch "$REF" --single-branch "$REPO" "$SRC_CHECKOUT"; then
            rm -rf "$SRC_CHECKOUT"
            die "clone of $REF from $REPO failed — check the ref and network"
        fi
    fi
    if [ ! -f "$SRC_CHECKOUT/rust-toolchain.toml" ]; then
        die "$SRC_CHECKOUT has no rust-toolchain.toml — not a tyu repo?"
    fi
    cd "$SRC_CHECKOUT"
    CHECKOUT_SHA="$(git rev-parse HEAD)"
    say "clone        : checkout $CHECKOUT_SHA"
fi

# ===========================================================================
# stage: build (the pinned nightly provisions itself via rust-toolchain.toml)
# ===========================================================================
if [ "$DRY_RUN" -eq 1 ]; then
    say "build        : cargo build --release -p langc -p tyu -p lmod-pack -p lmod-sign -p lmod-encrypt (provisions the rust-toolchain.toml pin)"
else
    say "build        : cargo build --release (host binary set)"
    if ! cargo build --release -p langc -p tyu -p lmod-pack -p lmod-sign -p lmod-encrypt; then
        die "source build failed — see the build log; the pinned nightly must build the release set (FR-5)"
    fi
fi

# ===========================================================================
# stage: install-bin (atomic move; existing binaries are only overwritten
#                     after a successful build)
# ===========================================================================
if [ "$DRY_RUN" -eq 1 ]; then
    say "install-bin  : mkdir -p $PREFIX/bin; install target/release/{${HOST_BIN_SET[*]}} -> $PREFIX/bin (atomic)"
else
    mkdir -p "$PREFIX/bin"
    for bin in "${HOST_BIN_SET[@]}"; do
        src="target/release/$bin"
        if [ ! -f "$src" ]; then
            die "expected binary $src missing after build"
        fi
        tmp="$PREFIX/bin/.${bin}.tmp.$$"
        cp -f "$src" "$tmp"
        chmod +x "$tmp"
        mv -f "$tmp" "$PREFIX/bin/$bin"
    done
    say "install-bin  : binaries installed into $PREFIX/bin"
fi

# ===========================================================================
# stage: record (written LAST — re-run safety and provenance)
# ===========================================================================
if [ "$DRY_RUN" -eq 1 ]; then
    say "record       : $PREFIX/INSTALL_RECORD (ref / checkout SHA / date / checkout path)"
else
    {
        echo "tyu install record (PLAN-RELEASE-1 S5)"
        echo "ref=$REF"
        echo "checkout_sha=$CHECKOUT_SHA"
        echo "date=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "checkout_path=$SRC_CHECKOUT"
        echo "prefix=$PREFIX"
        echo "binaries=${HOST_BIN_SET[*]}"
    } > "$PREFIX/INSTALL_RECORD"
fi

# ===========================================================================
# stage: advise
# ===========================================================================
if [ "$DRY_RUN" -eq 1 ]; then
    say "advise       : print PATH export, repo-anchored usage contract, tyu doctor, smoke commands"
    echo "dry-run: no side effects taken — exit 0."
    exit 0
fi

# PATH hygiene: warn (not fail) when an earlier PATH entry shadows $PREFIX/bin
if command -v tyu >/dev/null 2>&1; then
    shadowing="$(command -v tyu)"
    if [ "$shadowing" != "$PREFIX/bin/tyu" ]; then
        echo "  [warn] PATH entry earlier than \$PREFIX shadows the installed tyu: $shadowing" >&2
        echo "  [warn] export PATH=\"$PREFIX/bin:\$PATH\" (or remove the shadowing dir) so doctor/tyu resolve here" >&2
    fi
fi

echo ""
echo "TYU installed via source build into $PREFIX (INSTALL_RECORD written)."
echo ""
echo "Add to your shell profile:"
echo "  export PATH=\"$PREFIX/bin:\$PATH\""
echo ""
echo "Usage contract (Q4 — the toolchain is repo-anchored):"
echo "  * the installed checkout at $SRC_CHECKOUT is the repo anchor; the"
echo "    workspace root (runtime/ports/platforms) and cwd/sysroot resolve"
echo "    from it, so the first place to work is inside it, or"
echo "  * standalone project: export TYU_SYSROOT=$SRC_CHECKOUT/sysroot"
echo "    (TYU_SYSROOT is honored before cwd/sysroot; --sysroot=<dir> wins over both)"
echo ""
echo "Health check:  tyu doctor"
echo "Smoke green:   tyu build ci/smoke/hello.mod --out-dir=<out> && <out>/image.elf  (must exit 0)"
echo "Smoke red:     tyu build ci/smoke/bad.mod --out-dir=<out-bad> && <out-bad>/image.elf  (must exit 20 / CONTRACT_FAIL)"
echo ""