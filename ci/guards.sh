#!/usr/bin/env bash
# Test-delivery guard: CI invariant layer (G1-G13).
#
# Checks:
#   G1  Every crate with #[test] runs >0 tests.
#   G2  No test=false / harness=false hides a #[test] outside tests/.
#   G3  Execution-tests do not construct raw product QEMU command lines.
#   G4  (informational) Prints per-package executed-test counts.
#   G5  Generated runtime symtabs include every runtime export.
#   G6  Loader 52xx diagnostics do not collide with language/runtime trap codes.
#   G7  ARM device-loader text size stays within the 16 KiB budget when built.
#   G8  No Result<_, String> in crates/tyu/src.
#   G9  Pure host crates forbid unsafe_code.
#   G10 Loader decrypt path has no panic/unwrap/expect.
#   G11 Codegen backends do not byte-match primitive names.
#   G11b Codegen backends do not byte-match compound type names
#        (`Slice(...)` / `SliceMut(...)` / `RegionRef*`) — D-13 type-class
#        dispatch replaced it after Phase P2.
#   G12 Codegen functions stay <=250 lines.
#   G13 Host input-derived panic/unreachable sites are retired.
#   G14 Every discovered platform pack manifest carries the descriptor schema
#       stamp `schema = 2` (descriptor v2 — platform-descriptor-and-mmio-semantics).
#   G14b The x86 MMIO emulated-aperture size is descriptor-sourced; the old
#        hardcoded 0x10000 constant is gone (P3, FR-21).
#   G15 No raw MMIO base literals remain in product source (P4, FR-23).
#   G16 ARM on-device loader forces the Thumb bit before `blx` (P5 fix).
#   G17 P6 platform binding wired end-to-end (descriptor -> board table ->
#       loader E5220-24 + reloc apply).
#   G17b Board aperture table embedded (`__lang_platform_desc`) and decoded on-device.
#   G18 Trap 26 (`RegionExhausted`) is a registered IR trap with a diag claim
#       table entry — the allocator exhaustion path is nameable in diagnostics.
#   G18b P7 reference allocator words present per bare-metal target
#        (arm/riscv metal.trust words + x86 __region_* arrays) and trap 26
#        registered in the diag claim table.
#   G19 `.obl.json` extraction is byte-deterministic across runs (P2, FR-17).
#   G20 Verify corpus fully accounted against ci/verify-allowlist.txt (P8):
#       every open obligation justified; no stale allowlist entries.
#   G21 Every platform pack carries the `[verification]` grant and no retired
#       hand-declared `data_stack_slots` key (amended §6.4, E6403).
#   G22 Report honesty block agrees with the object on the corpus (FR-16,
#       guard form).
#   G23 NFR-10 doc gates: README doc map, ch03 both-policies example, ch04
#       contract obligations/elision, error-registry E6410/E6413.
#   G52 PLAN-RELEASE-1 S1: `.github` must stay tracked — the CI evidence
#       surface is repository truth, never an ignore line.
#   G53 PLAN-RELEASE-1 S2: every workflow job declares `timeout-minutes`.
#   G54 PLAN-RELEASE-1 S3: the toolchain pin is a dated nightly
#       (nightly-YYYY-MM-DD), not a floating `nightly`.
#   G55 PLAN-RELEASE-1 S9 (FR-7): a v* tag ref must be the canonical
#       three-component zero-patch form (vMAJOR.MINOR.0) with a changelog
#       section — delegated to ci/release-verify-tag.sh, the single
#       implementation of the tag-format check.
#
# Escape hatch: add `# guards: allow-no-tests` as a comment in the
# package's Cargo.toml to suppress G1/G2 for that package.  This is
# itself greppable and must be reviewed when the structural issue is
# fixed.
#
# Usage:
#   bash ci/guards.sh
#     Exits 0 on pass, non-zero on failure.
#
# Mutation checks (manual, documented):
#   1. Add `test = false` to crates/ir/Cargo.toml -> guard MUST fail.
#   2. Gate all lmod tests behind a bogus feature -> guard MUST fail.
#   3. Add `fn regression() -> Result<(), String> { Ok(()) }` under crates/tyu/src;
#      G8 MUST report it.
#   4. Remove `#![forbid(unsafe_code)]` from a Slice-4 crate after that slice;
#      G9 MUST report it.
#   5. Add `b"u8"` primitive dispatch to a codegen backend after Slice 6;
#      G11 MUST report it.
#   6. Add a >250-line function under crates/codegen-* after Slice 7;
#      G12 MUST report it.
#   7. Revert all mutations after check.
#   8. Remove `schema = 2` from one platform pack manifest after Phase P1;
#      G14 MUST report it.
#   9. Reintroduce a compound type-name byte-match
#      (`starts_with(b"Slice(")` / `== b"RegionRef"`) in a codegen backend
#      after Phase P2; G11 MUST report it.

set -euo pipefail

RED=1; GREEN=2; YELLOW=3
msg()  { local c=$1; shift; tput setaf "$c" 2>/dev/null || true; echo "  $*"; tput sgr0 2>/dev/null || true; }

failures=0
total_tests=0
counts=""

if [ ! -f Cargo.toml ] || ! grep -q '\[workspace\]' Cargo.toml 2>/dev/null; then
    msg $RED "Must be run from workspace root (no Cargo.toml with [workspace] found)"
    exit 1
fi

# Use python3 for JSON parsing (pre-installed on GitHub runners and most distros)
PYTHON=$(command -v python3 || command -v python || echo "")
if [ -z "$PYTHON" ]; then
    msg $RED "python3 is required but not found."
    exit 1
fi

# Get workspace metadata
metadata=$(cargo metadata --format-version 1 --no-deps 2>/dev/null) || {
    msg $RED "cargo metadata failed -- is the workspace valid?"
    exit 1
}

# Extract manifest paths
manifest_list=$("$PYTHON" -c "
import sys, json
data = json.load(sys.stdin)
for p in data['packages']:
    print(p['manifest_path'])
" <<< "$metadata" 2>/dev/null) || {
    msg $RED "Failed to parse cargo metadata"
    exit 1
}

while IFS= read -r manifest_path; do
    [ -z "$manifest_path" ] && continue
    pkg_dir=$(dirname "$manifest_path")
    pkg_name=$("$PYTHON" -c "
import sys, json
data = json.load(sys.stdin)
mp = '$manifest_path'
for p in data['packages']:
    if p['manifest_path'] == mp:
        print(p['name'])
        break
" <<< "$metadata" 2>/dev/null)

    local_src="$pkg_dir/src"
    local_tests="$pkg_dir/tests"

    # Check for escape hatch
    allow_no_tests=false
    if grep -q '# guards: allow-no-tests' "$manifest_path" 2>/dev/null; then
        allow_no_tests=true
    fi

    # Detect #[test] in source and test dirs (|| true keeps grep failure from
    # triggering set -e via pipefail inside $())
    has_test_anywhere=false
    has_test_in_source=false

    if [ -d "$local_src" ]; then
        count=$(grep -rl '#\[test\]' --include='*.rs' "$local_src" 2>/dev/null | head -1 | wc -l || true)
        if [ "$count" -gt 0 ]; then
            has_test_anywhere=true
            has_test_in_source=true
        fi
    fi
    if [ -d "$local_tests" ]; then
        count=$(grep -rl '#\[test\]' --include='*.rs' "$local_tests" 2>/dev/null | head -1 | wc -l || true)
        if [ "$count" -gt 0 ]; then
            has_test_anywhere=true
        fi
    fi

    if [ "$has_test_anywhere" = false ]; then
        counts="$counts|PKG $pkg_name: 0 tests (no #[test] found)"
        continue
    fi

    # --- G1: Zero-test guard ---
    # Sum test counts across ALL test targets (lib units, integration tests).
    # Uses the summary line "N tests, ..." from each target.
    # || true handles non-zero exit on compile failure.
    list_output=$(cargo test -p "$pkg_name" -- --list --color never 2>&1 || true)
    test_count=$(echo "$list_output" | awk '/^[0-9]+ tests,/ {total += $1} END {print total+0}' || echo "0")
    test_count=${test_count:-0}
    test_count=${test_count:-0}

    if [ "$test_count" -eq 0 ]; then
        if [ "$allow_no_tests" = true ]; then
            counts="$counts|PKG $pkg_name: 0 tests (ALLOWED: allow-no-tests)"
        else
            counts="$counts|PKG $pkg_name: 0 tests (FAIL)"
            msg $RED "  FAIL: $pkg_name has #[test] but runs 0 tests"
            failures=$((failures + 1))
        fi
    else
        total_tests=$((total_tests + test_count))
        counts="$counts|PKG $pkg_name: $test_count tests"
    fi

    # --- G2: test=false/harness=false with in-source #[test] ---
    # Only flag if there is a 'test = false' or 'harness = false' on a target
    # AND there is no [lib] target that hosts tests instead.
    if [ "$has_test_in_source" = true ]; then
        has_tests_via_lib=false
        if grep -q '^\[lib\]' "$manifest_path" 2>/dev/null; then
            # Extract the [lib] section and check for test=false (not just doctest=false)
            lib_section=$(awk '/^\[lib\]/{flag=1; next} /^\[/{flag=0} flag' "$manifest_path" 2>/dev/null || true)
            if ! echo "$lib_section" | grep -qE '(^|[^a-z])test\s*=\s*false'; then
                has_tests_via_lib=true
            fi
        fi

        if [ "$has_tests_via_lib" = false ]; then
            if grep -qE '^\s*(test|harness)\s*=\s*false' "$manifest_path" 2>/dev/null; then
                if [ "$allow_no_tests" = false ]; then
                    msg $RED "  FAIL: $pkg_name has test=false or harness=false but #[test] in source"
                    failures=$((failures + 1))
                fi
            fi
        fi
    fi
done < <(echo "$manifest_list")

# --- G3: raw product QEMU construction gate ---
# Execution-tests must exercise the product runner, not spawn qemu-system
# directly.  Keep any deliberate direct smoke coverage outside this path.
if grep -R -n -E 'Command::new\("qemu-system' crates/execution-tests/tests 2>/dev/null \
    | grep -v 'direct_qemu_smoke' \
    | grep -v 'runner.rs' >/dev/null; then
    msg $RED "  FAIL: execution-tests must not construct raw qemu-system command lines"
    failures=$((failures + 1))
fi

# --- G5: generated runtime symtab completeness ---
have_x86_dynamic_tools=true
for tool in cargo fasm ld nm; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        have_x86_dynamic_tools=false
    fi
done

if [ "$have_x86_dynamic_tools" = true ]; then
    guard_dir=$(mktemp -d "${TMPDIR:-/tmp}/tyu-guards-symtab.XXXXXX")
    cat > "$guard_dir/Main.mod" <<'EOF'
module Main;
: main ( -- i64 ) 0 ;
export { main };
end;
EOF
    if cargo build -q -p langc -p tyu >/dev/null 2>&1; then
        if target/debug/tyu build --mode=dynamic --target=x86_64-unknown-none \
            --sysroot="$(pwd)/sysroot" \
            --out-dir="$guard_dir/out" \
            "$guard_dir/Main.mod" >/dev/null 2>"$guard_dir/build.stderr"; then
            names_file="$guard_dir/out/lang_symtab.names"
            runtime_obj="$guard_dir/out/runtime.o"
            if [ ! -f "$names_file" ] || [ ! -f "$runtime_obj" ]; then
                msg $RED "  FAIL: symtab gate did not produce runtime.o + lang_symtab.names"
                failures=$((failures + 1))
            else
                nm "$runtime_obj" 2>/dev/null \
                    | awk '{print $NF}' \
                    | grep -E '^(w_[0-9a-f]{16}|__lang_|__stack_overflow)$' \
                    | sort -u > "$guard_dir/runtime.exports" || true
                awk '{print $2}' "$names_file" | sort -u > "$guard_dir/symtab.names"
                missing=$(comm -23 "$guard_dir/runtime.exports" "$guard_dir/symtab.names" || true)
                if [ -n "$missing" ]; then
                    msg $RED "  FAIL: generated .lang.symtab is missing runtime exports:"
                    echo "$missing" | while IFS= read -r sym; do msg $RED "    $sym"; done
                    failures=$((failures + 1))
                fi
            fi
        else
            msg $RED "  FAIL: symtab completeness build failed"
            sed 's/^/    /' "$guard_dir/build.stderr" >&2
            failures=$((failures + 1))
        fi
    else
        msg $RED "  FAIL: cargo build -p langc -p tyu failed for symtab gate"
        failures=$((failures + 1))
    fi
    rm -rf "$guard_dir"
else
    if [ "${CI:-}" ]; then
        msg $RED "  FAIL: symtab completeness gate requires cargo, fasm, ld, and nm under CI"
        failures=$((failures + 1))
    else
        msg $YELLOW "  WARN: skipping symtab completeness gate (missing cargo/fasm/ld/nm)"
    fi
fi

# --- G6: diagnostic band collision gate ---
collision_report=$("$PYTHON" - <<'PY'
import pathlib, re, sys
loader = pathlib.Path("crates/loader-core/src/error.rs").read_text()
claims = pathlib.Path("crates/diag-core/src/claims.rs").read_text()
loader_codes = {int(x) for x in re.findall(r'=>\s*(52\d\d)\b', loader)}
loader_codes |= {int(x) for x in re.findall(r'\b(52\d\d)\b', loader)}
trap_codes = {int(x) for x in re.findall(r'^\s*(\d+)\s*=>', claims, re.M)}
collisions = sorted(loader_codes & trap_codes)
if collisions:
    print(" ".join(map(str, collisions)))
    sys.exit(1)
PY
) || {
    msg $RED "  FAIL: loader 52xx codes collide with language/runtime trap codes: $collision_report"
    failures=$((failures + 1))
}

# --- G7: ARM loader size budget ---
# Accounting note (post-LTO): the loader builds with fat LTO + 1 CGU, which
# fuses loader-core/lmod/loader code into `device_loader_archive-` CGU
# objects — per-crate `loader_core-` members no longer exist. The gate
# therefore counts ALL archive .text EXCEPT the toolchain-provided runtime
# support (compiler_builtins/core/alloc), i.e. the code this build itself
# contributes — including the signing crypto when `--features signing`.
# Budget 32 KiB (measured 2026-09-27: 21,302 B signed, o3+LTO+1 CGU).
# The archive lives under the per-feature-set target dir (plain | signing |
# encryption | signing_encryption) — measure the SIGNED build when present
# (worst case: the crypto is the biggest loader text).
arm_archive="target/device-loader/signing/thumbv7m-none-eabi/release/libdevice_loader_archive.a"
[ -f "$arm_archive" ] || arm_archive="target/device-loader/plain/thumbv7m-none-eabi/release/libdevice_loader_archive.a"
if [ -f "$arm_archive" ]; then
    size_tool=$(command -v arm-none-eabi-size || command -v size || true)
    if [ -z "$size_tool" ]; then
        msg $RED "  FAIL: ARM loader size gate has an archive but no size tool"
        failures=$((failures + 1))
    else
        text_bytes=$("$size_tool" -A "$arm_archive" 2>/dev/null | awk '
            /^[^[:space:]].*\(ex / {
                name = $1
                sub(/-.*$/, "", name)
                in_own = (name == "device_loader_archive" || name == "loader_core" || name == "lmod")
                next
            }
            in_own && $1 ~ /^\.text/ {sum += $2}
            END {print sum+0}
        ')
        if [ "$text_bytes" -gt 32768 ]; then
            msg $RED "  FAIL: ARM device-loader .text is ${text_bytes} bytes (> 32768)"
            failures=$((failures + 1))
        else
            msg $GREEN "  ARM device-loader .text size: ${text_bytes} bytes"
        fi
    fi
else
    if [ "${CI:-}" ]; then
        msg $RED "  FAIL: ARM loader size gate requires $arm_archive under CI"
        failures=$((failures + 1))
    else
        msg $YELLOW "  WARN: skipping ARM loader size gate ($arm_archive not built)"
    fi
fi

# --- G8-G13: staged hardening gates ---
# These are informational in Slice 1. Each owning slice flips its check to a
# hard failure once the corresponding debt is removed.
tyu_string_results=$(grep -R -n -E 'Result<[^>]*, *String>' crates/tyu/src --include='*.rs' 2>/dev/null || true)
tyu_string_count=$(printf '%s\n' "$tyu_string_results" | sed '/^$/d' | wc -l | tr -d ' ')
if [ "$tyu_string_count" -eq 0 ]; then
    msg $GREEN "  G8: no Result<_, String> signatures in crates/tyu/src"
else
    msg $RED "  G8 FAIL: $tyu_string_count Result<_, String> signature(s) remain"
    printf '%s\n' "$tyu_string_results" >&2
    failures=$((failures + 1))
fi

missing_forbid=""
for crate in crates/ir crates/codegen-core crates/codegen-arm crates/codegen-riscv crates/codegen-x86_64 crates/verifier; do
    if ! grep -q '#!\[forbid(unsafe_code)\]' "$crate/src/lib.rs" 2>/dev/null; then
        missing_forbid="$missing_forbid $crate"
    fi
done
if [ -z "$missing_forbid" ]; then
    msg $GREEN "  G9: pure host crates carry #![forbid(unsafe_code)]"
else
    msg $RED "  G9 FAIL: missing forbid attrs:$missing_forbid"
    failures=$((failures + 1))
fi
pure_unsafe_hits=$(grep -R -n -E '\bunsafe[[:space:]]*(\{|fn|impl|trait)' \
    crates/ir/src crates/codegen-core/src crates/codegen-arm/src crates/codegen-riscv/src crates/codegen-x86_64/src \
    --include='*.rs' 2>/dev/null || true)
pure_unsafe_count=$(printf '%s\n' "$pure_unsafe_hits" | sed '/^$/d' | wc -l | tr -d ' ')
if [ "$pure_unsafe_count" -eq 0 ]; then
    msg $GREEN "  G9: pure host crates contain no unsafe constructs"
else
    msg $RED "  G9 FAIL: $pure_unsafe_count unsafe construct(s) in pure host crates"
    printf '%s\n' "$pure_unsafe_hits" >&2
    failures=$((failures + 1))
fi

decrypt_hits=$(awk '
    /^#\[cfg\(test\)\]/ {in_tests=1; in_region=0}
    in_tests {next}
    /parse_enc_header_view|decrypt|EncAuthFail|aad_region|payload_region/ {in_region=1}
    in_region && /unwrap\(\)|expect\(|panic!/ {print FILENAME ":" FNR ":" $0}
    /register_exports|make_exec|LoadedModule/ {in_region=0}
' crates/loader-core/src/load.rs 2>/dev/null || true)
decrypt_count=$(printf '%s\n' "$decrypt_hits" | sed '/^$/d' | wc -l | tr -d ' ')
if [ "$decrypt_count" -eq 0 ]; then
    msg $GREEN "  G10: loader decrypt region has no panic/unwrap/expect hits"
else
    msg $RED "  G10 FAIL: $decrypt_count panic/unwrap/expect hit(s) in decrypt-adjacent code"
    printf '%s\n' "$decrypt_hits" >&2
    failures=$((failures + 1))
fi

primitive_dispatch=$(grep -R -n -E 'b"(u8|u16|u32|u64|i8|i16|i32|i64|usize|isize|bool)"' \
    crates/codegen-arm/src crates/codegen-riscv/src crates/codegen-x86_64/src \
    --include='*.rs' 2>/dev/null || true)
primitive_count=$(printf '%s\n' "$primitive_dispatch" | sed '/^$/d' | wc -l | tr -d ' ')
if [ "$primitive_count" -eq 0 ]; then
    msg $GREEN "  G11: no backend primitive byte-string dispatch"
else
    msg $RED "  G11 FAIL: $primitive_count backend primitive byte-string match(es) remain"
    printf '%s\n' "$primitive_dispatch" >&2
    failures=$((failures + 1))
fi

compound_dispatch=$(grep -R -n -E 'starts_with\(b"Slice\(|starts_with\(b"SliceMut\(|starts_with\(b"RegionRef|== b"RegionRef|== b"RegionRefMut' \
    crates/codegen-arm/src crates/codegen-riscv/src crates/codegen-x86_64/src \
    --include='*.rs' 2>/dev/null || true)
compound_count=$(printf '%s\n' "$compound_dispatch" | sed '/^$/d' | wc -l | tr -d ' ')
if [ "$compound_count" -eq 0 ]; then
    msg $GREEN "  G11b: no backend compound type-name byte-match (D-13)"
else
    msg $RED "  G11b FAIL: $compound_count backend compound type-name match(es) remain (D-13)"
    printf '%s\n' "$compound_dispatch" >&2
    failures=$((failures + 1))
fi

long_codegen_functions=$("$PYTHON" - <<'PY'
import pathlib, re
for path in sorted(pathlib.Path("crates").glob("codegen-*/src/*.rs")):
    text = path.read_text()
    for m in re.finditer(r'(?m)^(\s*)(pub\s+)?fn\s+([A-Za-z0-9_]+)\b', text):
        start = text[:m.start()].count("\n")
        brace = text.find("{", m.end())
        if brace < 0:
            continue
        depth = 0
        end_pos = brace
        for i, ch in enumerate(text[brace:], brace):
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                if depth == 0:
                    end_pos = i
                    break
        end = text[:end_pos].count("\n")
        length = end - start + 1
        if length > 250:
            print(f"{path}:{start+1}:{m.group(3)}:{length}")
PY
)
long_fn_count=$(printf '%s\n' "$long_codegen_functions" | sed '/^$/d' | wc -l | tr -d ' ')
if [ "$long_fn_count" -eq 0 ]; then
    msg $GREEN "  G12: codegen functions are <=250 lines"
else
    msg $RED "  G12 FAIL: $long_fn_count codegen function(s) exceed 250 lines"
    printf '%s\n' "$long_codegen_functions" >&2
    failures=$((failures + 1))
fi

host_panic_hits=$(grep -R -n -E 'panic!|unreachable!' \
    crates/ir/src crates/semantics/src crates/codegen-core/src crates/verifier/src \
    --include='*.rs' 2>/dev/null || true)
host_panic_count=$(printf '%s\n' "$host_panic_hits" | sed '/^$/d' | wc -l | tr -d ' ')
if [ "$host_panic_count" -eq 0 ]; then
    msg $GREEN "  G13: host parse/type/core paths have no panic/unreachable hits"
else
    msg $RED "  G13 FAIL: $host_panic_count panic/unreachable hit(s) remain"
    printf '%s\n' "$host_panic_hits" >&2
    failures=$((failures + 1))
fi

# --- G14: descriptor schema stamp coverage (Phase P1, renewed P8, P12.1) ---
# Every discovered platform pack manifest (platforms/<name>/platform.toml and
# runtime/*.platform.toml) must carry the pack-manifest schema stamp. Since
# P12.1 (developer-proof-pipeline.md §6.7) that stamp is `schema = 3` (the
# additive [model] section); the descriptor *content* model stays v2 — the
# manifest stamp and the descriptor content schema are distinct versions.
# A pack without the stamp is a legacy pack the compiler cannot consume; a
# missing stamp is exactly the drift this spec retires. Mutation check #8.
missing_schema=""
for manifest in platforms/*/platform.toml runtime/*.platform.toml; do
    if [ ! -e "$manifest" ]; then
        continue
    fi
    if ! grep -qE '^schema[[:space:]]*=[[:space:]]*3' "$manifest"; then
        missing_schema="$missing_schema $manifest"
    fi
done
if [ -z "$missing_schema" ]; then
    msg $GREEN "  G14: all platform pack manifests carry the schema-3 stamp"
else
    msg $RED "  G14 FAIL: pack manifests missing 'schema = 3':$missing_schema"
    failures=$((failures + 1))
fi

# --- G14c: rp2350 datasheet facts are anchored (P8, renewed by the audit) ---
# Presence alone proved too weak: the first P8 draft passed this guard with
# RP2040-era bases and IRQ numbers. The guard now anchors load-bearing
# datasheet values (RP-008373-DS-2 Tables 13/14/95); the full row-level pin
# lives in crates/tyu/tests/rp2350_datasheet_facts.rs (docs-as-tests).
rp2350_devices=$(grep -c '^\[\[platform.devices\]\]' platforms/rp2350/platform.toml || true)
rp2350_irqs=$(grep -c 'irq = ' platforms/rp2350/platform.toml || true)
rp2350_ok=1
# Correct RP2350 anchors (DS2): IO_BANK0 base_offset 0x28000, UART0 0x70000,
# TIMER0 0xb0000, QMI/XIP_QMI 0xd0000; SRAM is 520 kB = 0x82000.
for anchor in 'base_offset = 0x28000' 'base_offset = 0x70000' 'base_offset = 0xb0000' \
              'base_offset = 0xd0000' 'length = 0x00082000'; do
    if ! grep -qF "$anchor" platforms/rp2350/platform.toml; then
        msg $RED "  G14c FAIL: rp2350 descriptor missing datasheet anchor: $anchor"
        rp2350_ok=0
    fi
done
# RP2040-era values must be gone entirely.
for stale in '0x14000' '0x34000' '0x54000' '0x60000' '0x00084000' 'PADS_BANK0_BASE, 0x4001C000' 'SIO_GPIO_OUT_SET,0x014'; do
    if grep -rqF "$stale" platforms/rp2350/; then
        msg $RED "  G14c FAIL: rp2350 pack carries RP2040-era value: $stale"
        rp2350_ok=0
    fi
done
# IRQ numbers: Table 95 values must appear (GPIO 21, UART0 33, SPI0 31).
for irq in 'irq = 21' 'irq = 33' 'irq = 31' 'irq = 36'; do
    if ! grep -qF "$irq" platforms/rp2350/platform.toml; then
        msg $RED "  G14c FAIL: rp2350 descriptor missing IRQ anchor: $irq"
        rp2350_ok=0
    fi
done
if [ "$rp2350_ok" -eq 1 ] && [ "$rp2350_devices" -ge 10 ] && [ "$rp2350_irqs" -ge 8 ]; then
    msg $GREEN "  G14c: rp2350 descriptor anchors the RP2350 datasheet (devices=$rp2350_devices irq_rows=$rp2350_irqs)"
else
    if [ "$rp2350_ok" -eq 1 ]; then
        msg $RED "  G14c FAIL: rp2350 datasheet tables incomplete (devices=$rp2350_devices irqs=$rp2350_irqs)"
    fi
    failures=$((failures + 1))
fi

# --- G14d: mem-width fixture coverage is target-broad (P8, FR-9) ---
# The memory-width fixture was x86-only (P1); P8 broadens it to arm/riscv and
# the arm variant exercises @u8/@u16/@u32/@u64. A regression that drops a
# width class breaks the corresponding fixture's width coverage.
for f in mem_width_arm mem_width_riscv; do
    if [ ! -e "crates/execution-tests/fixtures/$f.mod" ]; then
        msg $RED "  G14d FAIL: missing mem-width fixture $f.mod"
        failures=$((failures + 1))
    fi
done
arm_widths=$(grep -cE '@u8|@u16|@u32|@i64|!u8|!u16|!u32|!i64' crates/execution-tests/fixtures/mem_width_arm.mod || true)
if [ "$arm_widths" -ge 4 ]; then
    msg $GREEN "  G14d: mem-width fixtures cover u8/u16/u32/u64 on arm+riscv (arm_widths=$arm_widths)"
else
    msg $RED "  G14d FAIL: mem_width_arm width coverage dropped (arm_widths=$arm_widths)"
    failures=$((failures + 1))
fi

# --- G14b: x86 MMIO aperture size is descriptor-sourced (P3, FR-21) ---
# The old hardcoded `0x10000`/`65536` emulated-aperture constant must not
# return to the x86 mmio lowering OR the Executable-mode `__mmio_mem` BSS
# reservation; both sizes now come from the backend's aperture table (target
# defaults or the compiled platform descriptor). A reservation smaller than
# the bounds-checked size admits MMIO past the array (finding F1).
mmio_const_hits=$(grep -R -n -E '0x10000|65536' \
    crates/codegen-x86_64/src/mmio.rs 2>/dev/null || true)
mmio_reserve_hits=$(grep -R -n -E '__mmio_mem rb [0-9]' \
    crates/codegen-x86_64/src/postlude.rs 2>/dev/null || true)
mmio_const_hits="${mmio_const_hits}${mmio_reserve_hits}"
mmio_const_count=$(printf '%s\n' "$mmio_const_hits" | sed '/^$/d' | wc -l | tr -d ' ')
if [ "$mmio_const_count" -eq 0 ]; then
    msg $GREEN "  G14b: x86 MMIO aperture size is descriptor-sourced (no 0x10000 constant)"
else
    msg $RED "  G14b FAIL: hardcoded emulated-aperture size returned to mmio.rs:"
    printf '%s\n' "$mmio_const_hits" >&2
    failures=$((failures + 1))
fi

# --- G15: no raw MMIO base literals in product source (P4, FR-23) ---
# After the P4 migration every register-map instance binds `board.<instance>`;
# a raw `MAP @ 0x…` base in source is the pre-symbolic pattern this slice
# retires. Product dirs only (fixtures/tests are migrated too and included).
raw_mmio_hits=$(grep -R -n -E '@[[:space:]]*0x[0-9a-fA-F]+' \
    crates/execution-tests/fixtures crates/execution-tests/tests \
    crates/tooling-tests/tests sysroot platforms runtime \
    --include='*.mod' --include='*.rs' 2>/dev/null || true)
# Exclude matches that are not MMIO instantiations (e.g. hex literals in
# comments/strings are fine; the pattern is `= MAP @ 0x`).
raw_mmio_count=$(printf '%s\n' "$raw_mmio_hits" | grep -E '= .* @[[:space:]]*0x' | sed '/^$/d' | wc -l | tr -d ' ' || true)
if [ "$raw_mmio_count" -eq 0 ]; then
    msg $GREEN "  G15: no raw MMIO base literals remain in product source (P4)"
else
    msg $RED "  G15 FAIL: raw MMIO base literals remain:"
    printf '%s\n' "$raw_mmio_hits" | grep -E '= .* @[[:space:]]*0x' >&2 || true
    failures=$((failures + 1))
fi

# --- G16: ARM on-device loader forces the Thumb bit before `blx` (P5 fix) ---
# The loader resolves module functions to an even `code_base`; Cortex-M is
# Thumb-only, so a `blx` to an even address switches to (nonexistent) ARM
# state → `v7M INVSTATE`. The dynamic-entry glue must OR the Thumb bit in, and
# the loader must stamp it on module function addresses by construction.
thumb_orr=$(grep -B1 "blx r0" platforms/armv7m-unknown-none/metal/dynamic_entry.asm | grep -c "orr r0, r0, #1" || true)
loader_thmb_hits=$(grep -c "module_fn_addr" crates/loader-core/src/load.rs || true)
if [ "$thumb_orr" -ge 1 ] && [ "$loader_thmb_hits" -ge 2 ]; then
    msg $GREEN "  G16: ARM on-device loader forces the Thumb bit before blx (INVSTATE fix)"
else
    msg $RED "  G16 FAIL: ARM dynamic loader Thumb-bit protection missing (blx to even address faults)"
    failures=$((failures + 1))
fi

# --- G17: P6 platform binding is wired end-to-end ---
# The steel thread (design doc §5.8): backends emit `__lang_aperture_{id}_base`
# reloc sites; lmod-pack records MmioApertureBase relocs without baking bases;
# tyu embeds the board aperture table (`__lang_platform_desc`); the loader
# enforces platform_hash (E5220), resolves+binds apertures (E5221/22/23), and
# writes the board base into reloc sites at load (FR-15). A missing link means
# a module silently binds the wrong geometry or the board identity is dropped.
bind_decls=$(grep -h -c "bind = \"arm-thumb-ldr-literal\"\|bind = \"riscv-hi20-lo12\"" platforms/*/platform.toml 2>/dev/null | awk '{s+=$1} END {print s+0}')
backend_sites=$(grep -h -c "__lang_aperture_" crates/codegen-arm/src/word.rs crates/codegen-riscv/src/word.rs | awk '{s+=$1} END {print s+0}')
board_table=$(grep -h -c "encode_board_table_blob\|__lang_platform_desc\|aperture_capability" crates/tyu/src/build.rs crates/codegen-core/src/compiled_desc.rs | awk '{s+=$1} END {print s+0}')
loader_bind=$(grep -h -c "bind_apertures\|PlatformHashMismatch\|ApertureConflict\|ApertureUnresolved\|ApertureTableMalformed\|ModinfoVersionUnsupported" crates/loader-core/src/load.rs crates/loader-core/src/apertures.rs crates/loader-core/src/error.rs | awk '{s+=$1} END {print s+0}')
reloc_apply=$(grep -h -c "apply_base" crates/loader-core/src/load.rs | awk '{s+=$1} END {print s+0}')
if [ "$bind_decls" -ge 2 ] && [ "$backend_sites" -ge 2 ] && [ "$board_table" -ge 3 ] && [ "$loader_bind" -ge 5 ] && [ "$reloc_apply" -ge 1 ]; then
    msg $GREEN "  G17: P6 platform binding wired end-to-end (descriptor -> board table -> loader E5220-24 + reloc apply)"
else
    msg $RED "  G17 FAIL: P6 platform binding incomplete (bind_decls=$bind_decls backend_sites=$backend_sites board_table=$board_table loader_bind=$loader_bind reloc_apply=$reloc_apply)"
    failures=$((failures + 1))
fi

# --- G17b: the board table blob is embedded as `__lang_platform_desc` and the
# device loader decodes it with the shared codec (design doc §5.8) ---
blob_producer=$(grep -h -c "encode_into(&mut buf, cd.platform_hash" crates/tyu/src/build.rs | awk '{s+=$1} END {print s+0}')
blob_consumer=$(grep -h -c "decode as decode_board_table\|__lang_platform_desc_start" crates/loader-core/src/boot.rs | awk '{s+=$1} END {print s+0}')
if [ "$blob_producer" -ge 1 ] && [ "$blob_consumer" -ge 1 ]; then
    msg $GREEN "  G17b: board aperture table embedded (__lang_platform_desc) and decoded on-device"
else
    msg $RED "  G17b FAIL: board table embed/decode broken (producer=$blob_producer consumer=$blob_consumer)"
    failures=$((failures + 1))
fi

# --- G18: trap 26 (RegionExhausted) registered in IR + diag claims ---
# `RegionExhausted` (trap 26) is a distinct IR trap code with its own text
# record, raised by the platform region allocator on exhaustion. (A host-side
# "set-payload" crypto subsystem that formerly shared this code was removed as
# unintended scope: it had no callers, no CLI surface, and no on-device
# counterpart.)
trap26_ir=$(grep -h -c "RegionExhausted" crates/ir/src/lib.rs | awk '{s+=$1} END {print s+0}')
trap26_claim=$(grep -c '26 => "REGION_EXHAUSTED"' crates/diag-core/src/claims.rs 2>/dev/null || echo 0)
if [ "$trap26_ir" -ge 2 ] && [ "$trap26_claim" -ge 1 ]; then
    msg $GREEN "  G18: trap 26 (RegionExhausted) registered in IR + diag claims"
else
    msg $RED "  G18 FAIL: trap 26 registration incomplete (ir=$trap26_ir claims=$trap26_claim)"
    failures=$((failures + 1))
fi

# --- G18b: P7 reference allocator words per bare-metal target + diag claim ---
# The `platform.mem.region-*` words must be real on every QEMU-capable target
# (x86: codegen-inline mmio/bump; ARM/RISC-V: metal.trust asm words in the metal
# runtime), and trap 26 must be registered in the diag claim table (not
# UNKNOWN_TRAP_CODE). This is the "no reference allocator in any target" fix.
region_words_arm=$(grep -c "w_7a5f795caa045668" platforms/armv7m-unknown-none/metal/runtime.asm runtime/armv7m-unknown-none/runtime.asm | awk -F: '{s+=$2} END {print s+0}')
region_words_rv=$(grep -c "w_7a5f795caa045668" platforms/riscv32-unknown-none/metal/runtime.asm runtime/riscv32-unknown-none/runtime.asm | awk -F: '{s+=$2} END {print s+0}')
trust_arm=$(grep -c "platform.mem.region-create" platforms/armv7m-unknown-none/platform.toml | awk '{s+=$1} END {print s+0}')
trust_rv=$(grep -c "platform.mem.region-create" platforms/riscv32-unknown-none/platform.toml | awk '{s+=$1} END {print s+0}')
claim26=$(grep -c "26 => \"REGION_EXHAUSTED\"" crates/diag-core/src/claims.rs | awk '{s+=$1} END {print s+0}')
x86_region=$(grep -c "__region_next" platforms/x86_64-unknown-none/metal/runtime.asm runtime/x86_64-unknown-none/runtime.asm | awk -F: '{s+=$2} END {print s+0}')
if [ "$region_words_arm" -ge 2 ] && [ "$region_words_rv" -ge 2 ] && [ "$trust_arm" -ge 1 ] && [ "$trust_rv" -ge 1 ] && [ "$claim26" -ge 1 ] && [ "$x86_region" -ge 2 ]; then
    msg $GREEN "  G18b: P7 reference allocator words present per target (arm=$region_words_arm rv=$region_words_rv trust=$trust_arm/$trust_rv claim26=$claim26 x86=$x86_region)"
else
    msg $RED "  G18b FAIL: P7 reference allocator incomplete (arm=$region_words_arm rv=$region_words_rv trust=$trust_arm/$trust_rv claim26=$claim26 x86=$x86_region)"
    failures=$((failures + 1))
fi

# --- G19: `.obl.json` extraction is byte-deterministic (P2, FR-17) ---
# Two runs of `langc --emit=obligations` over the same source must produce
# byte-identical artifacts (no clocks, no env, no iteration-order output).
# Also: the reference fixture's artifact is committed as a golden and must not
# drift silently (the exact-bytes suite in crates/langc/tests enforces this
# under cargo; this gate re-checks determinism through the built binary).
if command -v cargo >/dev/null 2>&1; then
    g19_dir=$(mktemp -d "${TMPDIR:-/tmp}/tyu-guards-obl.XXXXXX")
    cat > "$g19_dir/Bank.mod" <<'EOF'
module Bank;
subtype Percent = i64 range 0..100;
subtype Counter = i64 range 0..1000000;
: clamp ( i64 -- Percent )
  dup 100 > [ drop 100 ] [ ] if
  dup 0 < [ drop 0 ] [ ] if
  as Percent
;
: bounded_inc ( Percent -- Percent ) 1 + as Percent ;
: main ( -- Counter ) 50 as Percent bounded_inc as Counter ;
end;
EOF
    if cargo build -q -p langc >/dev/null 2>&1; then
        mkdir -p "$g19_dir/a" "$g19_dir/b"
        if target/debug/langc --emit=obligations --out-dir="$g19_dir/a" "$g19_dir/Bank.mod" \
           && target/debug/langc --emit=obligations --out-dir="$g19_dir/b" "$g19_dir/Bank.mod" \
           && cmp -s "$g19_dir/a/Bank.obl.json" "$g19_dir/b/Bank.obl.json"; then
            msg $GREEN "  G19: obl.json extraction is byte-deterministic (FR-17)"
        else
            msg $RED "  G19 FAIL: --emit=obligations is not byte-deterministic"
            failures=$((failures + 1))
        fi
    else
        msg $RED "  G19 FAIL: cargo build -p langc failed"
        failures=$((failures + 1))
    fi
    rm -rf "$g19_dir"
else
    if [ "${CI:-}" ]; then
        msg $RED "  G19 FAIL: determinism gate requires cargo under CI"
        failures=$((failures + 1))
    else
        msg $YELLOW "  WARN: skipping obl determinism gate (no cargo)"
    fi
fi

echo ""
# --- G20: verify corpus accounted against the allowlist (slice P8) ---
# Every OPEN obligation across the `ci/verify-corpus` builds must be
# justified in `ci/verify-allowlist.txt` (id<TAB>reason); the gate fails on
# any open id NOT in the list, and on any list entry that no longer occurs
# (self-cleaning — a stale entry means the justification died and must be
# re-derived). The corpus is built the way a user builds: hosted target,
# default features, `--verify=on`.
g20_fail=0
if command -v python3 >/dev/null 2>&1 && command -v cargo >/dev/null 2>&1; then
    if cargo build -q -p tyu >/dev/null 2>&1; then
        g20_dir=$(mktemp -d "${TMPDIR:-/tmp}/tyu-guards-g20.XXXXXX")
        allowlist="ci/verify-allowlist.txt"
        seen=""
        corpus_bad=0
        for mod in ci/verify-corpus/*.mod; do
            name=$(basename "$mod" .mod)
            out="$g20_dir/$name"
            if target/debug/tyu build --target=x86_64-unknown-linux-gnu --out-dir="$out" "$mod" >/dev/null 2>&1; then
                : # build ok; inspect report below
            else
                msg $RED "  G20 FAIL: corpus module '$name' did not build"
                corpus_bad=1
                continue
            fi
            python3 - "$out/verify-report.json" "$allowlist" "$name" <<'PYEOF'
import json, sys
report, allowlist, name = sys.argv[1], sys.argv[2], sys.argv[3]
d = json.load(open(report))
opens = [o["id"] for o in d["open"]]
allowed = set()
with open(allowlist) as f:
    for line in f:
        line = line.rstrip("\n")
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if parts:
            allowed.add(parts[0])
unjustified = [i for i in opens if i not in allowed]
print("G20 corpus", name, "open", len(opens))
for i in unjustified:
    print("UNJUSTIFIED", i)
sys.exit(1 if unjustified else 0)
PYEOF
            rc=$?
            if [ $rc -ne 0 ]; then
                msg $RED "  G20 FAIL: '$name' has open obligations not in the allowlist"
                g20_fail=1
            fi
            seen="$seen
$(python3 - "$out/verify-report.json" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
for o in d["open"]:
    print(o["id"])
PYEOF
)"
        done
        if [ "$corpus_bad" -ne 0 ]; then
            g20_fail=1
        fi
        # Self-cleaning: every allowlist id must still occur in the corpus.
        stale=""
        while IFS= read -r line; do
            case "$line" in
                ""|\#*) continue ;;
            esac
            id="${line%%$'\t'*}"
            if ! printf '%s' "$seen" | grep -qx "$id"; then
                stale="$stale $id"
            fi
        done < "$allowlist"
        if [ -n "$stale" ]; then
            msg $RED "  G20 FAIL: stale allowlist entries (no longer open in the corpus):$stale"
            g20_fail=1
        fi
        rm -rf "$g20_dir"
        if [ "$g20_fail" -eq 0 ]; then
            msg $GREEN "  G20: verify corpus fully accounted against ci/verify-allowlist.txt"
        fi
    else
        msg $YELLOW "  WARN: skipping G20 (cargo build -p tyu failed)"
    fi
else
    if [ "${CI:-}" ]; then
        msg $RED "  G20 FAIL: corpus gate requires cargo+python3 under CI"
        g20_fail=1
    else
        msg $YELLOW "  WARN: skipping G20 (no cargo/python3)"
    fi
fi
failures=$((failures + g20_fail))

# --- G21: platform packs carry the `[verification]` geometry grant (P8) ---
# Every in-tree platform pack must declare the ISR bounded-stack grant
# (`isr_stack_slots`, the one *declared* budget — amended §6.4), and must NOT
# carry the retired hand-declared `data_stack_slots` key (E6403 rejects it at
# parse — the migrated packs must be clean so the lint has nothing to hide).
g21_fail=0
for manifest in platforms/*/platform.toml runtime/*.platform.toml; do
    if [ ! -e "$manifest" ]; then
        continue
    fi
    if ! grep -q '^\s*\[verification\]' "$manifest" || ! grep -q '^\s*isr_stack_slots' "$manifest"; then
        msg $RED "  G21 FAIL: '$manifest' lacks the [verification] isr_stack_slots grant"
        g21_fail=1
    fi
    if grep -q '^\s*data_stack_slots' "$manifest"; then
        msg $RED "  G21 FAIL: '$manifest' still declares the retired hand-declared data_stack_slots key (E6403)"
        g21_fail=1
    fi
done
[ "$g21_fail" -eq 0 ] && msg $GREEN "  G21: every platform pack carries the [verification] grant (no retired data_stack_slots)"
failures=$((failures + g21_fail))

# --- G22: the report names the truth (FR-16, guard form, P8) ---
# The open-cast corpus module's report must agree with the object: the
# retained out-of-range constant trap is exactly one subtype check and a
# provably-failing record (a discharge here would remove a trap that must
# fire — the honest block says so, and the hosted image keeps its data-stack
# guards).
g22_fail=0
if command -v python3 >/dev/null 2>&1 && command -v cargo >/dev/null 2>&1; then
    if cargo build -q -p tyu >/dev/null 2>&1; then
        g22_dir=$(mktemp -d "${TMPDIR:-/tmp}/tyu-guards-g22.XXXXXX")
        if target/debug/tyu build --target=x86_64-unknown-linux-gnu --out-dir="$g22_dir" \
            ci/verify-corpus/open-cast.mod >/dev/null 2>&1; then
            python3 - "$g22_dir/verify-report.json" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
em = d["emitted_checks"]
assert em["subtype_range"] == 1, f"open-cast must retain exactly one subtype trap: {em}"
assert em["data_stack_guards"] is True, "hosted image keeps its data-stack guards"
pf = [p["id"] for p in d["provably_failing"]]
assert any("main::subtype-range::0" in pid for pid in pf), f"report must name the provably-failing cast: {pf}"
opens = [o["id"] for o in d["open"]]
assert any("main::subtype-range::0" in oid for oid in opens), "the retained trap is an open obligation"
PYEOF
            g22_rc=$?
            if [ $g22_rc -eq 0 ]; then
                msg $GREEN "  G22: report honesty block agrees with the object (FR-16, guard form)"
            else
                msg $RED "  G22 FAIL: report/object bijection broken on the verified corpus"
                g22_fail=1
            fi
        else
            msg $RED "  G22 FAIL: open-cast corpus module did not build"
            g22_fail=1
        fi
        rm -rf "$g22_dir"
    else
        msg $YELLOW "  WARN: skipping G22 (cargo build -p tyu failed)"
    fi
else
    if [ "${CI:-}" ]; then
        msg $RED "  G22 FAIL: honesty gate requires cargo+python3 under CI"
        g22_fail=1
    else
        msg $YELLOW "  WARN: skipping G22 (no cargo/python3)"
    fi
fi
failures=$((failures + g22_fail))

# --- G23: NFR-10 doc gates (P8) ---
# The verification docs are registered and teach the boundary as the norm:
# README maps the design-doc suite, the book teaches the both-policies
# example (ch03) and contract obligations/elision (ch04), and the error
# registry names the artifact band codes.
g23_fail=0
if ! grep -q "ir-op-semantics" README.md \
   || ! grep -q "verification-obligations" README.md \
   || ! grep -q "static-verification" README.md; then
    msg $RED "  G23 FAIL: README doc map must register the verification design docs"
    g23_fail=1
fi
if ! grep -q 'E6410' docs/book/appendix-b-error-registry.md \
   || ! grep -q 'E6413' docs/book/appendix-b-error-registry.md; then
    msg $RED "  G23 FAIL: error-registry appendix must carry E6410/E6413"
    g23_fail=1
fi
if ! grep -q "Compile-time discharge" docs/book/ch03-types.md \
   || ! grep -q "no-open" docs/book/ch03-types.md; then
    msg $RED "  G23 FAIL: ch03 must teach the both-policies compile-time-discharge example"
    g23_fail=1
fi
if ! grep -q "Contract obligations and elision" docs/book/ch04-contracts.md \
   || ! grep -q "module-loading" docs/book/ch04-contracts.md; then
    msg $RED "  G23 FAIL: ch04 must teach contract obligations and dynamic-export retention"
    g23_fail=1
fi
[ "$g23_fail" -eq 0 ] && msg $GREEN "  G23: NFR-10 doc gates (doc map, both-policies example, errors, contracts)"
failures=$((failures + g23_fail))

# --- G24: PLAN-VERIFY-3 P1.1 — canonical-statement boundary gates ---
# The statement encoder is the hash the whole pipeline binds against and MUST
# be SHA-256-only (FR-14): fnv1a64 stays confined to identity keys (id_hash),
# and no new serde may enter the stmt/codec surface (FR-15). These greps are
# the enforcement point; the property tests in
# crates/verifier/tests/stmt_encoder.rs cover the encoder behavior.
g24_fail=0
if grep -rn "serde" crates/verifier/src/stmt.rs >/dev/null 2>&1; then
    msg $RED "  G24 FAIL: serde leaked into crates/verifier/src/stmt.rs (hand-rolled JSON only, FR-15)"
    g24_fail=1
fi
# Only actual fnv CALL/IDENTIFIER forms count — the module takes the
# no-usage position in prose, but a real `fnv1a64(...)` invocation (or a
# fnv-hash identifier) is the smuggling the gate exists to catch.
if grep -rniE 'fnv1a64[[:space:]]*\(|fnv1a::|fnv_hash' crates/verifier/src/stmt.rs >/dev/null 2>&1; then
    msg $RED "  G24 FAIL: fnv hashing invoked in crates/verifier/src/stmt.rs (SHA-256 only, FR-14)"
    g24_fail=1
fi
if [ "$(grep -c 'Sha256' crates/verifier/src/stmt.rs 2>/dev/null || echo 0)" -lt 1 ]; then
    msg $RED "  G24 FAIL: crates/verifier/src/stmt.rs must construct SHA-256 digests"
    g24_fail=1
fi
# FR-14 guard form: integrity digests introduced by P1 are SHA-256. The
# statement encoder must never invoke a non-SHA-256 hash.
if grep -rn 'fnv1a64[[:space:]]*(' crates/verifier/tests/stmt_encoder.rs >/dev/null 2>&1; then
    msg $RED "  G24 FAIL: statement-encoder tests must not invoke fnv hashing"
    g24_fail=1
fi
[ "$g24_fail" -eq 0 ] && msg $GREEN "  G24: statement encoder is SHA-256-only (fnv1a64 confined to identity keys)"
failures=$((failures + g24_fail))

# --- G25: PLAN-VERIFY-3 P1.3 — statement-golden surface ---
# Statement hashes are pinned per triple over the corpus
# (`tooling-tests/tests/statement_goldens.rs` regenerates/compares; the band
# rule §Q4 item 3 is enforced by that test's byte comparison). This static
# gate pins the *surface*: every triple carries exactly the corpus golden set
# (no module-collision clobbering — the store keys on `<Module>.stmt.json`),
# and the gate test exists. The byte-stable comparison runs under
# `cargo test --workspace` and the labeled CI step.
g25_fail=0
if [ ! -f crates/tooling-tests/tests/statement_goldens.rs ]; then
    msg $RED "  G25 FAIL: statement-goldens test missing"
    g25_fail=1
fi
# The exact corpus set (distinct modules per triple): verify-corpus modules
# + the authored-intent / casts inline fixtures.
expected_set="Bank Clean Contract EventLoop Lending OpenCast"
for triple in x86_64-unknown-linux-gnu x86_64-unknown-none armv7m-unknown-none riscv32-unknown-none; do
    dir="test-goldens/statements/$triple"
    if [ ! -d "$dir" ]; then
        msg $RED "  G25 FAIL: statement-golden directory missing for $triple"
        g25_fail=1
        continue
    fi
    actual=$(cd "$dir" && find . -maxdepth 1 -name '*.stmt.json' -printf '%f\n' 2>/dev/null | sed 's/\.stmt\.json$//' | sort | tr '\n' ' ' | sed 's/ $//')
    if [ "$actual" != "$expected_set" ]; then
        msg $RED "  G25 FAIL: statement-golden set for $triple drifted"
        msg $RED "    expected: $expected_set"
        msg $RED "    actual:   ${actual:-<none>}"
        g25_fail=1
    fi
done
[ "$g25_fail" -eq 0 ] && msg $GREEN "  G25: statement goldens exact-set present for all four triples (band rule §Q4)"
failures=$((failures + g25_fail))

# --- G26: PLAN-VERIFY-3 P2.1 — verifier layering (no codegen-core) ---
# The verifier is the semantic kernel: `ir → verifier → (semantics, langc) →
# tyu`. It must NOT import the backend's target identity — it carries its own
# four-field `TargetSpec` duplicate, and parity with codegen-core is asserted
# by `tooling-tests/tests/target_parity.rs` (the one place in the tree that
# may compare them, because tooling-tests already depends on both). This gate
# makes the boundary mechanical: a future `use codegen_core` in the verifier
# fails CI, and the parity test pins the values.
g26_fail=0
if grep -rnE 'codegen_core|codegen-core' crates/verifier/ >/dev/null 2>&1; then
    # Note: `target_parity.rs` lives in tooling-tests, never in crates/verifier/.
    msg $RED "  G26 FAIL: verifier must not reference codegen-core (layering rule)"
    g26_fail=1
fi
if [ ! -f crates/tooling-tests/tests/target_parity.rs ]; then
    msg $RED "  G26 FAIL: target-parity test missing (verifier TargetSpec vs codegen-core)"
    g26_fail=1
fi
[ "$g26_fail" -eq 0 ] && msg $GREEN "  G26: verifier stays codegen-core-free; target parity pinned by tooling-tests"
failures=$((failures + g26_fail))

# --- G27: PLAN-VERIFY-3 P2.2 — per-target vector corpus surface ---
# The conformance vectors (`tyu.vec/1`) exist for every recognized triple,
# the vector-corpus + interval-targets tests exist, and the committed files
# carry the schema tag. Byte-stability is enforced by
# `vector_corpus_files_match_regeneration` under `cargo test --workspace`;
# this gate pins the *surface* (files + tag + tests present), like G25.
g27_fail=0
if [ ! -f crates/verifier/tests/vector_corpus.rs ] || [ ! -f crates/verifier/tests/interval_targets.rs ]; then
    msg $RED "  G27 FAIL: vector corpus tests missing (vector_corpus.rs / interval_targets.rs)"
    g27_fail=1
fi
for triple in x86_64-unknown-linux-gnu x86_64-unknown-none armv7m-unknown-none riscv32-unknown-none; do
    idx="crates/verifier/test-vectors/$triple/index.json"
    if [ ! -f "$idx" ]; then
        msg $RED "  G27 FAIL: vector corpus missing for $triple ($idx)"
        g27_fail=1
        continue
    fi
    if ! grep -q '"tyu.vec/1"' "$idx"; then
        msg $RED "  G27 FAIL: $idx lacks the tyu.vec/1 schema tag"
        g27_fail=1
    fi
done
[ "$g27_fail" -eq 0 ] && msg $GREEN "  G27: per-target vector corpus (tyu.vec/1) present for all four triples"
failures=$((failures + g27_fail))

# --- G32: PLAN-VERIFY-3 P4.2 — T-C substrate surface ---
# T-C (the first registry theorem) must be *present as a theorem,* not as a
# label: the concrete semantics file, the stack-algebra file with the
# registry theorem names, the axiom-audit surface, the stackmeta goldens
# (the empirical hook), and the exporting tooling-test must all exist. The
# *proving* is the blocking port gate (`ci/port.sh`); this gate pins the
# surface in the regular Rust CI.
g32_fail=0
PORT_DIR=verification/ports/lean
for f in Tyu/Step.lean Tyu/Sound.lean AxiomAudit.lean; do
    if [ ! -f "$PORT_DIR/$f" ]; then
        msg $RED "  G32 FAIL: $PORT_DIR/$f missing (P4 substrate)"
        g32_fail=1
    fi
done
if ! grep -q "theorem stack_algebra_sequence" "$PORT_DIR/Tyu/Sound.lean" || \
   ! grep -q "theorem stack_algebra_walk" "$PORT_DIR/Tyu/Sound.lean" || \
   ! grep -q "theorem stepOk_length" "$PORT_DIR/Tyu/Sound.lean"; then
    msg $RED "  G32 FAIL: T-C registry theorem names missing from Tyu/Sound.lean"
    g32_fail=1
fi
if [ ! -f crates/tooling-tests/tests/stackmeta_export.rs ]; then
    msg $RED "  G32 FAIL: stackmeta exporter test missing (T-C empirical hook)"
    g32_fail=1
fi
sm_count=0
for triple in x86_64-unknown-linux-gnu x86_64-unknown-none armv7m-unknown-none riscv32-unknown-none; do
    for f in test-goldens/stackmeta/"$triple"/*.json; do
        [ -f "$f" ] && sm_count=$((sm_count + 1))
    done
done
if [ "$sm_count" -lt 20 ]; then
    msg $RED "  G32 FAIL: stackmeta goldens thin ($sm_count files; expected ≥ 20 across four triples)"
    g32_fail=1
fi
[ "$g32_fail" -eq 0 ] && msg $GREEN "  G32: T-C substrate (Step + Sound theorems, axiom audit, stackmeta goldens + exporter)"
failures=$((failures + g32_fail))

# --- G40: PLAN-VERIFY-3 P11 — verify_manifest + loader policy surface ---
# The in-module verification anchor (P11) must be present as live surface:
# the record codec (lmod), the pack writer (lmod-pack), the loader
# validation + policy hook (E6500/6501/6502 in loader-core), the deploy
# pairing gate (E6510 in tyu), and their tests. Behaviour is exercised by
# the test suites; this gate pins the surface in the regular Rust CI.
g40_fail=0
if [ ! -f crates/lmod/src/verify_manifest.rs ]; then
    msg $RED "  G40 FAIL: verify_manifest record codec missing (crates/lmod/src/verify_manifest.rs)"
    g40_fail=1
fi
for c in VerifyManifestMalformed VerifyManifestDigest VerifyPolicyReject; do
    if ! grep -q "$c" crates/loader-core/src/error.rs; then
        msg $RED "  G40 FAIL: LoadError::$c missing (E6500/6501/6502)"
        g40_fail=1
    fi
done
if ! grep -q "fn verify_policy" crates/loader-core/src/platform.rs; then
    msg $RED "  G40 FAIL: LoaderPlatform::verify_policy hook missing"
    g40_fail=1
fi
if ! grep -q "fn validate_verify_manifest" crates/loader-core/src/load.rs; then
    msg $RED "  G40 FAIL: loader verify_manifest validation missing"
    g40_fail=1
fi
if ! grep -q "VerifyPairing" crates/tyu/src/error.rs || ! grep -q "E6510" crates/tyu/src/deploy.rs; then
    msg $RED "  G40 FAIL: deploy pairing (E6510) missing"
    g40_fail=1
fi
[ "$g40_fail" -eq 0 ] && msg $GREEN "  G40: verify_manifest surface (codec + loader policy + deploy pairing)"
failures=$((failures + g40_fail))

# --- G28: PLAN-VERIFY-3 §P0 — workspace bar: rustfmt ---
# Every slice ends `cargo fmt --check` green (§P0 workspace bar). The gate is
# the check itself — mechanical, zero judgement, no bypass path short of
# editing this gate.
g28_fail=0
fmt_out="$(cargo fmt --check 2>&1 || true)"
if [ -n "$fmt_out" ]; then
    if printf '%s\n' "$fmt_out" | grep -q "^Diff in"; then
        msg $RED "  G28 FAIL: rustfmt drift — run 'cargo fmt' (workspace bar, §P0)"
        printf '%s\n' "$fmt_out" | grep "^Diff in" | head -5 | while read -r d; do
            msg $RED "    $d"
        done
    else
        # Not drift — a broken fmt invocation (e.g. a missing component on a
        # minimal-profile toolchain). Show it: a guard that cannot explain
        # its failure is a diagnosis hole.
        msg $RED "  G28 FAIL: cargo fmt errored (not drift):"
        printf '%s\n' "$fmt_out" | head -3 | while read -r d; do
            msg $RED "    $d"
        done
    fi
    g28_fail=1
fi
[ "$g28_fail" -eq 0 ] && msg $GREEN "  G28: rustfmt clean (workspace bar)"
failures=$((failures + g28_fail))

# --- G29: PLAN-VERIFY-3 §P0 — workspace bar: clippy -D warnings ---
# `--all-targets` (strictly stronger than --lib --bins --tests: it also covers
# examples/benches if any ever appear). Historical note (resolved 2026-09-26):
# --all-targets synthesizes a bin-as-test target even for bins marked
# `test = false`, and that phantom collided with the no_std runtime —
# langc's phantom linked std, whose panic_impl duplicated hosted-rt's
# #[panic_handler] (E0152), and lang-assemble's (`harness = false`) was built
# as a plain no_std binary that cannot satisfy the test profile's unwind
# strategy. Resolution: the no_std runtime lang items are owned by each
# binary root behind `cfg(not(test))` (hosted-rt exposes the plain
# `hosted_rt::panic` routine they delegate to), and lang-assemble dropped the
# redundant `harness = false`. The phantom is check-only — `test = false`
# keeps `cargo test` from ever building or running it — and the real
# freestanding binaries stay `#![no_std]` + panic=abort. Warnings are denied;
# suppressions require a justification comment at the item (spec Rule: no
# silent suppression).
g29_fail=0
if ! cargo clippy --workspace --all-targets -- -D warnings >/dev/null 2>&1; then
    msg $RED "  G29 FAIL: clippy -D warnings (workspace bar, §P0)"
    cargo clippy --workspace --all-targets 2>&1 | grep "^ *-->" | head -5 | while read -r d; do
        msg $RED "    $d"
    done
    g29_fail=1
fi
[ "$g29_fail" -eq 0 ] && msg $GREEN "  G29: clippy -D warnings clean (--all-targets)"
failures=$((failures + g29_fail))

# --- G30: PLAN-VERIFY-3 P3.1/P3.2 — Lean port surface ---
# The first port's generated data layer + conformance runner are *material*
# contracts of the developer-proof pipeline: the committed generated files
# must carry the drift-lock and the completeness marker, the exporter test
# must exist (`TYU_EXPORT_PORTS=1 cargo test -p verifier --test export_drift`
# regenerates; committed files byte-compare otherwise), and the port gate
# (`ci/port.sh`) must be present and executable. The *port build itself* is
# the blocking `ci/port.sh` job (separate CI tier — needs Lean); this gate
# pins the surface in the regular Rust CI.
g30_fail=0
if [ ! -f crates/verifier/tests/export_drift.rs ]; then
    msg $RED "  G30 FAIL: port drift-lock test missing (crates/verifier/tests/export_drift.rs)"
    g30_fail=1
fi
if [ ! -x ci/port.sh ]; then
    msg $RED "  G30 FAIL: ci/port.sh missing or not executable (chmod +x ci/port.sh)"
    g30_fail=1
fi
PORT_DIR=verification/ports/lean
for f in lean-toolchain lakefile.toml Tyu.lean \
         Tyu/IR/Op.lean Tyu/IR/Semantics.lean Tyu/IR/Target.lean Tyu/Mem.lean Main.lean; do
    if [ ! -f "$PORT_DIR/$f" ]; then
        msg $RED "  G30 FAIL: $PORT_DIR/$f missing (port package surface)"
        g30_fail=1
    fi
done
if ! grep -q "src/export/lean.rs" "$PORT_DIR/Tyu/IR/Op.lean" || ! grep -q "DO NOT EDIT" "$PORT_DIR/Tyu/IR/Op.lean"; then
    msg $RED "  G30 FAIL: generated files lack the don't-edit drift-lock marker"
    g30_fail=1
fi
[ "$g30_fail" -eq 0 ] && msg $GREEN "  G30: Lean port surface (generated data layer + drift lock + port.sh)"
failures=$((failures + g30_fail))

# --- G31: PLAN-VERIFY-3 §P0/§Q11 — port hygiene ---
# The port's *authored* conjectures are forbidden: no `sorry`, `Admitted`,
# or `native_decide` (permitted axioms only propext/Quot.sound/
# Classical.choice; sorryAx/Lean.ofReduceBool never — §Q11 item 2). The
# generated completeness theorem must be present (`SEMANTICS_total`), and a
# hand-edited generated file dropping a row must fail the port build — the
# marker grep verifies the theorem text is present (the lake build gate is
# the blocking job).
g31_fail=0
if grep -rnE 'sorry|Admitted|native_decide' "$PORT_DIR" --include='*.lean' >/dev/null 2>&1; then
    msg $RED "  G31 FAIL: forbidden proof placeholder (sorry/Admitted/native_decide) in $PORT_DIR"
    grep -rnE 'sorry|Admitted|native_decide' "$PORT_DIR" --include='*.lean' | head -5
    g31_fail=1
fi
if ! grep -q "theorem SEMANTICS_total" "$PORT_DIR/Tyu/IR/Semantics.lean"; then
    msg $RED "  G31 FAIL: SEMANTICS_total completeness theorem missing from $PORT_DIR/Tyu/IR/Semantics.lean"
    g31_fail=1
fi
[ "$g31_fail" -eq 0 ] && msg $GREEN "  G31: port hygiene (no sorry/Admitted/native_decide; SEMANTICS_total present)"
failures=$((failures + g31_fail))


# --- G33: PLAN-VERIFY-3 P5 — Gen renderer + golden-statement surface ---
# The statement renderer is the interface a developer's proofs bind to
# (§Q4): the renderer exe, the semantics glue, the committed golden
# statements (elaborated by the port gate), the corpus artifacts, and the
# Rust-side drift test must all exist. The byte-stability + hash agreement
# run under `cargo test --workspace` and `ci/port.sh`; this gate pins the
# surface.
g33_fail=0
PORTDIR=verification/ports/lean
for f in Tyu/Gen/Stmt.lean Tyu/Gen/Render.lean Tyu/Gen/Sha256.lean          Tyu/Gen/Golden.lean GenMain.lean goldens/obl/Bank.obl.json; do
    if [ ! -f "$PORTDIR/$f" ]; then
        msg $RED "  G33 FAIL: $PORTDIR/$f missing (P5 renderer surface)"
        g33_fail=1
    fi
done
for m in Bank Clean Contract EventLoop Lending LoopSub OpenCast Post; do
    if [ ! -f "$PORTDIR/Tyu/Gen/Golden/$m.lean" ]; then
        msg $RED "  G33 FAIL: golden statements missing for $m"
        g33_fail=1
    fi
    if [ ! -f "$PORTDIR/goldens/gen/$m.gen.json" ]; then
        msg $RED "  G33 FAIL: gen metadata missing for $m"
        g33_fail=1
    fi
done
if [ ! -f crates/tooling-tests/tests/gen_render_drift.rs ]; then
    msg $RED "  G33 FAIL: gen-render drift test missing (crates/tooling-tests/tests/gen_render_drift.rs)"
    g33_fail=1
fi
if grep -rnE 'sorry|Admitted|native_decide' "$PORTDIR/Tyu/Gen" --include='*.lean' >/dev/null 2>&1; then
    msg $RED "  G33 FAIL: forbidden placeholder (sorry/Admitted/native_decide) in Tyu/Gen"
    g33_fail=1
fi
[ "$g33_fail" -eq 0 ] && msg $GREEN "  G33: P5 statement-renderer surface (Gen + goldens + drift test)"
failures=$((failures + g33_fail))

# --- G34: PLAN-VERIFY-3 P6 — the developer-proof pipeline surface ---
# `tyu proof init`/`fill`, the generated `.tyu-verify/lean/` package
# (vendored port lib + generated Gen + lakefile + harvest stub), the E6418
# Gen-digest gate, the `--verify-tool` pass-through, the honest report v2
# `proof` section, and the two-tier integration tests must all exist.
g34_fail=0
if [ ! -f crates/tyu/src/proof.rs ]; then
    msg $RED "  G34 FAIL: crates/tyu/src/proof.rs missing (P6 package pipeline)"
    g34_fail=1
fi
# CLI surface: proof init/fill + --verify-tool + dispatch.
grep -q 'ProofArgs::Init' crates/tyu/src/args.rs || { msg $RED "  G34 FAIL: proof init unparsed"; g34_fail=1; }
grep -q 'ProofArgs::Fill' crates/tyu/src/args.rs || { msg $RED "  G34 FAIL: proof fill unparsed"; g34_fail=1; }
grep -q 'parse_proof' crates/tyu/src/args.rs || { msg $RED "  G34 FAIL: proof subcommand parser missing"; g34_fail=1; }
grep -q 'Command::Proof' crates/tyu/src/main.rs || { msg $RED "  G34 FAIL: proof dispatch missing in main.rs"; g34_fail=1; }
grep -q 'verify-tool=' crates/tyu/src/args.rs || { msg $RED "  G34 FAIL: --verify-tool unparsed"; g34_fail=1; }
grep -q 'VerifyTool::Lean' crates/tyu/src/args.rs || { msg $RED "  G34 FAIL: lean tool enum missing"; g34_fail=1; }
# Package generation + gates + the obligation-artifact E6418 digest check.
grep -q 'lean_package_path' crates/tyu/src/proof.rs || { msg $RED "  G34 FAIL: package path helper missing"; g34_fail=1; }
grep -q 'E6418' crates/tyu/src/proof.rs || { msg $RED "  G34 FAIL: E6418 Gen-digest gate missing"; g34_fail=1; }
grep -q 'harvest-not-built' crates/tyu/src/proof.rs || { msg $RED "  G34 FAIL: harvest boundary missing"; g34_fail=1; }
grep -q 'TYU_SKIP_PORT_BUILD' crates/tyu/src/proof.rs || { msg $RED "  G34 FAIL: skip-mode hook missing"; g34_fail=1; }
grep -q 'proof_files_hash' crates/tyu/src/proof.rs || { msg $RED "  G34 FAIL: proof-files hash missing"; g34_fail=1; }
# langc pass-through + report v2 proof section.
grep -q 'verify_tool' crates/langc/src/args.rs || { msg $RED "  G34 FAIL: langc --verify-tool unparsed"; g34_fail=1; }
grep -q 'tyu.verify-report/v2' crates/verifier/src/report.rs || { msg $RED "  G34 FAIL: report schema v2 missing"; g34_fail=1; }
grep -q 'proof' crates/verifier/src/codec.rs || { msg $RED "  G34 FAIL: report proof section unencoded"; g34_fail=1; }
# Tests: the two P6 integration suites + the build wiring.
for t in crates/tyu/tests/proof_init.rs crates/tyu/tests/build_verify_integration.rs; do
    if [ ! -f "$t" ]; then
        msg $RED "  G34 FAIL: $t missing"
        g34_fail=1
    fi
done
grep -q 'run_lean_pipeline' crates/tyu/src/build.rs || { msg $RED "  G34 FAIL: pipeline not wired into build.rs"; g34_fail=1; }
[ "$g34_fail" -eq 0 ] && msg $GREEN "  G34: P6 developer-proof pipeline surface (proof init/fill, package, E6418, report v2, tests)"
failures=$((failures + g34_fail))

# --- G35: PLAN-VERIFY-3 P6 hardening — §7.2 slot key, toolchain E6416 tests,
# tier-A CI wiring ---
# Three review findings closed in P6: (a) the verdicts-cache slot key must
# carry the §7.2 environment components (semantics/stmt/toolchain/model/
# proof-files) so a proof-file or model change can never silently reuse stale
# (P7 harvested) verdicts; (b) the toolchain missing/mismatch E6416 paths are
# unit-tested hermetically (check_lean_toolchain); (c) the toolchain-present
# tier-A build-integration test is wired into the port gate so it runs when
# a recognized-port toolchain is present.
g35_fail=0
for tok in 'VerifyEnvKey' 'verdicts_slot_name' 'parse_any_verdicts_name' 'proof_files_hash' 'check_lean_toolchain' 'ensure_lean_toolchain'; do
    if ! grep -q "$tok" crates/tyu/src/proof.rs; then
        msg $RED "  G35 FAIL: proof.rs lacks $tok (P6 hardening surface)"
        g35_fail=1
    fi
done
if ! grep -q 'verify_env' crates/tyu/src/build.rs; then
    msg $RED "  G35 FAIL: build.rs does not thread the §7.2 verify env"
    g35_fail=1
fi
if ! grep -q 'TYU_PROOF_E2E' ci/port.sh || ! grep -q 'build_verify_integration' ci/port.sh; then
    msg $RED "  G35 FAIL: the P6 tier-A test is not wired into ci/port.sh"
    g35_fail=1
fi
[ "$g35_fail" -eq 0 ] && msg $GREEN "  G35: P6 hardening (§7.2 slot key + toolchain E6416 tests + tier-A wiring)"
failures=$((failures + g35_fail))

# --- G36: PLAN-VERIFY-3 P7 — harvest, verdicts v2, proven policy ---
# The kernel-checked path: the Lean harvest library (vendored into generated
# packages), the v2 verdicts codec (closed registries, E6417, producer
# downgrade, proof_ref retired), the two-pass build consuming harvested
# verdicts, the `proven` policy (Q12 trust gate through langc + report
# failure), the report trust×method×surface + TCB sections, and the harvest
# fixture/golden wired into the port gate.
g36_fail=0
for f in Tyu/Verdicts/Harvest.lean Tyu/Gen/Render.lean; do
    if [ ! -f "verification/ports/lean/$f" ]; then
        msg $RED "  G36 FAIL: port file $f missing (P7 harvest surface)"
        g36_fail=1
    fi
done
if grep -rnE 'sorry|Admitted|native_decide' verification/ports/lean/Tyu/Verdicts/Harvest.lean >/dev/null 2>&1; then
    msg $RED "  G36 FAIL: sorry/Admitted/native_decide in the harvest library"
    g36_fail=1
fi
if ! grep -q 'tyu.axiom-audit/1' verification/ports/lean/Tyu/Verdicts/Harvest.lean; then
    msg $RED "  G36 FAIL: harvest axiom-audit evidence missing (tyu.axiom-audit/1)"
    g36_fail=1
fi
if ! grep -q 'axiom_audit' crates/tyu/src/proof.rs \
   || ! grep -q 'harvest_error_code' crates/tyu/src/proof.rs; then
    msg $RED "  G36 FAIL: harvest audit re-home / typed error surface missing (proof.rs)"
    g36_fail=1
fi
for tok in 'tyu.verdicts/v2' 'Trust' 'Method' 'proof_ref' 'restrict_to_recognized' 'UnknownMethod'; do
    if ! grep -q "$tok" crates/verifier/src/verdict.rs; then
        msg $RED "  G36 FAIL: verdicts v2 codec lacks $tok"
        g36_fail=1
    fi
done
for tok in 'run_harvest' 'harvest_entry' 'VerifyEnvKey' 'verdicts_override'; do
    if ! grep -q "$tok" crates/tyu/src/proof.rs crates/tyu/src/build.rs 2>/dev/null; then
        msg $RED "  G36 FAIL: two-pass harvest wiring lacks $tok"
        g36_fail=1
    fi
done
if ! grep -q 'VerifyPolicy::Proven\|"proven"' crates/tyu/src/args.rs; then
    msg $RED "  G36 FAIL: proven policy unparsed"
    g36_fail=1
fi
if ! grep -q 'proven_gate' crates/semantics/src/typecheck/irgen/mod.rs; then
    msg $RED "  G36 FAIL: langc proven trust gate missing"
    g36_fail=1
fi
if ! grep -q 'trust' crates/verifier/src/report.rs || ! grep -q 'tcb' crates/verifier/src/report.rs; then
    msg $RED "  G36 FAIL: report trust/TCB sections missing"
    g36_fail=1
fi
if [ ! -f verification/ports/lean/tests/harvest-fixture/hvharvest.lean ] \
   || [ ! -f test-goldens/harvest/Tiny.verdicts.v2.json ]; then
    msg $RED "  G36 FAIL: harvest fixture/golden missing"
    g36_fail=1
fi
if ! grep -q 'P7.1 harvest gate' ci/port.sh; then
    msg $RED "  G36 FAIL: the harvest gate is not wired into ci/port.sh"
    g36_fail=1
fi
# P7.3 gates (the tooling-tests suites + the port tamper negatives + the
# error-registry rows): the consumption-side tamper matrix and the proven
# trust gate are material deliverables with their own gates.
for t in crates/tooling-tests/tests/policy_proven.rs crates/tooling-tests/tests/tamper_matrix.rs; do
    if [ ! -f "$t" ]; then
        msg $RED "  G36 FAIL: $t missing (P7.3 consumption gates)"
        g36_fail=1
    fi
done
for s in verification/ports/lean/tests/tamper/run-delete-theorem.sh \
         verification/ports/lean/tests/tamper/run-mutate-gen.sh; do
    if [ ! -x "$s" ]; then
        msg $RED "  G36 FAIL: $s missing or not executable (P7.1 tamper negatives)"
        g36_fail=1
    fi
done
for code in 6416 6417 6418 6419 6420 6421; do
    if ! grep -q "E$code" docs/book/appendix-b-error-registry.md; then
        msg $RED "  G36 FAIL: appendix-b error registry missing E$code (P6/P7 allocation, §6.9)"
        g36_fail=1
    fi
done
[ "$g36_fail" -eq 0 ] && msg $GREEN "  G36: P7 harvest + verdicts v2 + proven policy surface"
failures=$((failures + g36_fail))

# --- G37: PLAN-VERIFY-3 P8 — assumption closure (T-CL) surface ---
# The T-CL registry statement must be *present as a theorem*, not a label:
# the abstract verdict-set structure + the registry theorems in the port,
# the axiom-audit + REVIEW.md §3 entries, the Rust image walker
# (`tyu::closure`) wired into the report composition, the report's
# per-image closure section, and the P8 test surfaces (assumption goldens,
# closure fixtures, harvest-merge). Proving/executing are the blocking gates
# (`ci/port.sh` axiom audit, the tooling suites under `cargo test
# --workspace`); this gate pins the surface in the regular Rust CI.
g37_fail=0
PORT_DIR=verification/ports/lean
if ! grep -q "namespace AssumptionClosure" "$PORT_DIR/Tyu/Sound.lean" \
   || ! grep -q "theorem transitive_closure_sound" "$PORT_DIR/Tyu/Sound.lean" \
   || ! grep -q "theorem cyclic_not_well_closed" "$PORT_DIR/Tyu/Sound.lean" \
   || ! grep -q "theorem open_edge_not_well_closed" "$PORT_DIR/Tyu/Sound.lean"; then
    msg $RED "  G37 FAIL: T-CL theorems missing from Tyu/Sound.lean (P8.1 surface)"
    g37_fail=1
fi
for thm in transitive_closure_sound cyclic_not_well_closed open_edge_not_well_closed \
           runtime_terminal no_edges_terminal; do
    if ! grep -q "AssumptionClosure.$thm" "$PORT_DIR/AxiomAudit.lean"; then
        msg $RED "  G37 FAIL: AxiomAudit.lean missing Tyu.Sound.AssumptionClosure.$thm"
        g37_fail=1
    fi
    if ! grep -q "AssumptionClosure.$thm" "$PORT_DIR/REVIEW.md"; then
        msg $RED "  G37 FAIL: REVIEW.md §3 missing Tyu.Sound.AssumptionClosure.$thm"
        g37_fail=1
    fi
done
if ! grep -q "### T-CL" devdocs/plans/design-doc/formal-semantics-core.md; then
    msg $RED "  G37 FAIL: formal-semantics-core.md T-CL registry entry missing"
    g37_fail=1
fi
if ! grep -q 'id: "T-CL"' crates/verifier/src/report.rs \
   || ! grep -q 'status: "theorem"' crates/verifier/src/report.rs; then
    msg $RED "  G37 FAIL: report TCB must carry T-CL with status theorem"
    g37_fail=1
fi
if ! grep -q "pub struct ClosureStatus" crates/verifier/src/report.rs \
   || ! grep -q '\\"closure\\":{' crates/verifier/src/codec.rs \
   || ! grep -q "pub fn check_image_closure" crates/tyu/src/closure.rs \
   || ! grep -q "check_image_closure" crates/tyu/src/verify.rs; then
    msg $RED "  G37 FAIL: the closure walker/report/encode wiring incomplete"
    g37_fail=1
fi
for t in crates/tooling-tests/tests/assumption_goldens.rs \
         crates/tooling-tests/tests/closure_fixtures.rs \
         crates/tyu/tests/harvest_merge.rs; do
    if [ ! -f "$t" ]; then
        msg $RED "  G37 FAIL: $t missing (P8 gates)"
        g37_fail=1
    fi
done
# P8.2 hardening: the closure runs BETWEEN harvest and pass-2 (the
# pre-codegen resolution — never an elided check with an open report row),
# and langc surfaces an open record's witness in the echo.
if ! grep -q "apply_harvest_closure" crates/tyu/src/closure.rs \
   || ! grep -q "apply_harvest_closure" crates/tyu/src/proof.rs; then
    msg $RED "  G37 FAIL: the pre-pass-2 harvest-closure adjustment is not wired (proof.rs)"
    g37_fail=1
fi
# P5 renderer scope: the canonical dispatches on the formula op (mmio
# OffsetLE + contract PredicateHolds), the contract/fmmio statements have
# semantics, and the renderer-scope regression test exists — the mmio
# encoder-hash lock unblocks register-map modules (E6418).
if ! grep -q '\"OffsetLE\"' "$PORT_DIR/Tyu/Gen/Render.lean" \
   || ! grep -q '\"PredicateHolds\"' "$PORT_DIR/Tyu/Gen/Render.lean" \
   || ! grep -q "def predicateHolds" "$PORT_DIR/Tyu/Gen/Stmt.lean" \
   || ! grep -q "def offsetWithin" "$PORT_DIR/Tyu/Gen/Stmt.lean"; then
    msg $RED "  G37 FAIL: the statement canonical/formula dispatch incomplete (mmio/contract statements)"
    g37_fail=1
fi
if [ ! -f crates/tooling-tests/tests/renderer_scope.rs ]; then
    msg $RED "  G37 FAIL: renderer-scope regression test missing (renderer_scope.rs)"
    g37_fail=1
fi
for f in test-goldens/assumptions/Bank.json test-goldens/assumptions/App.json \
         test-goldens/assumptions/App-degraded.json; do
    if [ ! -f "$f" ] || ! grep -q '"tyu.assumptions/1"' "$f"; then
        msg $RED "  G37 FAIL: $f missing or lacks the tyu.assumptions/1 tag"
        g37_fail=1
    fi
done
[ "$g37_fail" -eq 0 ] && msg $GREEN "  G37: P8 assumption-closure surface (T-CL theorem + walker + report + goldens)"
failures=$((failures + g37_fail))

# --- G38: P9 source-fragment embedding + T-S transcription surface ---
# The pure-fragment source semantics (Tyu/Src.lean) exists, the T-S registry
# theorem is proven + axiom-audited, the TCB reports it `theorem`, the
# renderer emits source-surface statements (src_stmt_*) with the src_def
# metadata, the harvest binds them with surface/relies, the Rust
# cross-surface interpreter + e2e fixture exist, and ci/port.sh runs the
# source-surface gate.
g38_fail=0
for f in "$PORT_DIR/Tyu/Src.lean" \
         crates/verifier/src/src_interp.rs \
         crates/verifier/tests/cross_surface.rs \
         crates/tooling-tests/tests/source_surface_e2e.rs \
         "$PORT_DIR/tests/source-fixture/SumFix.lean" \
         "$PORT_DIR/tests/source-fixture/Sum.obl.json"; do
    if [ ! -f "$f" ]; then
        msg $RED "  G38 FAIL: $f missing (P9 surface)"
        g38_fail=1
    fi
done
if ! grep -q "theorem transcription" "$PORT_DIR/Tyu/Src.lean"; then
    msg $RED "  G38 FAIL: T-S theorem missing from Tyu/Src.lean"
    g38_fail=1
fi
if ! grep -q "Tyu.Sound.transcription" "$PORT_DIR/AxiomAudit.lean"; then
    msg $RED "  G38 FAIL: the T-S theorem is not axiom-audited"
    g38_fail=1
fi
if ! grep -q 'id: "T-S"' crates/verifier/src/report.rs \
   || ! grep -q 'status: "theorem"' crates/verifier/src/report.rs; then
    msg $RED "  G38 FAIL: report TCB must carry T-S with status theorem (P9.1 exit)"
    g38_fail=1
fi
if ! grep -q "src_stmt_" "$PORT_DIR/Tyu/Gen/Render.lean" \
   || ! grep -q "srcClassify" "$PORT_DIR/Tyu/Gen/Render.lean" \
   || ! grep -q "src_def" "$PORT_DIR/Tyu/Verdicts/Harvest.lean"; then
    msg $RED "  G38 FAIL: the source-surface statement rendering or harvest binding is incomplete"
    g38_fail=1
fi
if ! grep -q '"relies"' crates/verifier/src/verdict.rs; then
    msg $RED "  G38 FAIL: the proof.relies surface is missing from the verdict codec"
    g38_fail=1
fi
# fragment boundary honesty: no `sorry`/`Admitted`/`native_decide` in the
# source embedding or the worked-example proof.
if grep -rn "sorry\|Admitted\|native_decide" "$PORT_DIR/Tyu/Src.lean" \
    "$PORT_DIR/tests/source-fixture/SumFix.lean" 2>/dev/null; then
    msg $RED "  G38 FAIL: forbidden placeholder in the source embedding / fixture proof"
    g38_fail=1
fi
if ! grep -q "P9 source-surface" ci/port.sh; then
    msg $RED "  G38 FAIL: the P9 source-surface gate is not wired into ci/port.sh"
    g38_fail=1
fi
[ "$g38_fail" -eq 0 ] && msg $GREEN "  G38: P9 source-fragment semantics (Tyu/Src), T-S theorem + audit, TCB status, source-surface renderer/harvest + cross-surface vectors + e2e + port gate"
failures=$((failures + g38_fail))

# --- G39: P9.3 fragment-vector conformance corpus (the Lean↔Rust pin) ---
# The pure-fragment surface now has the IR surface's mechanical pin: a
# committed `tyu.fragvec/1` corpus (program → trace) generated by
# `fragment_vectors.rs` from canonical `--emit=ir` text, executed by
# `cross_surface.rs` on the Rust side and by the port
# (`Tyu/Conformance/Fragment.lean` + `conformance --level fragment`) — a
# fragment-op drift on either side diverges the committed traces. The corpus
# must be present, the Rust generation/execution tests must exist, the port
# must have the runner + exe mode, and ci/port.sh must run the gate.
g39_fail=0
for f in crates/verifier/test-vectors/fragment/index.json \
         crates/verifier/tests/fragment_vectors.rs \
         "$PORT_DIR/Tyu/Conformance/Fragment.lean"; do
    if [ ! -f "$f" ]; then
        msg $RED "  G39 FAIL: $f missing (P9.3 fragment corpus)"
        g39_fail=1
    fi
done
if ! grep -q '"schema":"tyu.fragvec/1"' crates/verifier/test-vectors/fragment/index.json 2>/dev/null \
   && ! grep -q '"schema": "tyu.fragvec/1"' crates/verifier/test-vectors/fragment/index.json 2>/dev/null; then
    msg $RED "  G39 FAIL: the fragment corpus lacks the tyu.fragvec/1 schema header"
    g39_fail=1
fi
if ! grep -q "fragment_corpus_agrees" crates/verifier/tests/cross_surface.rs; then
    msg $RED "  G39 FAIL: cross_surface.rs must execute the committed fragment corpus"
    g39_fail=1
fi
if ! grep -q "fragment_corpus_covers_every_fragment_op" crates/verifier/tests/fragment_vectors.rs; then
    msg $RED "  G39 FAIL: fragment_vectors.rs must pin per-op coverage"
    g39_fail=1
fi
if ! grep -q '"--level" :: "fragment"' "$PORT_DIR/Main.lean"; then
    msg $RED "  G39 FAIL: the conformance exe lacks the --level fragment mode"
    g39_fail=1
fi
if ! grep -q "parseFragFile\|runFragFile" "$PORT_DIR/Tyu/Conformance/Fragment.lean"; then
    msg $RED "  G39 FAIL: the port's fragment runner is incomplete"
    g39_fail=1
fi
if ! grep -q "P9.3 fragment" ci/port.sh; then
    msg $RED "  G39 FAIL: the P9.3 fragment gate is not wired into ci/port.sh"
    g39_fail=1
fi
[ "$g39_fail" -eq 0 ] && msg $GREEN "  G39: P9.3 fragment-vector corpus (tyu.fragvec/1): Rust generator + cross_surface execution + port runner + port gate — the Lean↔Rust fragment pin"
failures=$((failures + g39_fail))

# G44: PLAN-VERIFY-3 P10 — the automation surface (tactic library, candidate
# pipeline, attribution, deploy knob) with the §P0 discipline gates.
g44_fail=0
[ -d "$PORT_DIR/Tyu/Automation" ] || { msg $RED "  G44 FAIL: missing Tyu/Automation library"; g44_fail=1; }
grep -q 'elab "tyu_auto"' "$PORT_DIR/Tyu/Automation/Auto.lean" || { msg $RED "  G44 FAIL: tyu_auto dispatcher missing"; g44_fail=1; }
grep -q 'elab "auto_cycle"' "$PORT_DIR/Tyu/Automation/Cycle.lean" || { msg $RED "  G44 FAIL: auto_cycle missing"; g44_fail=1; }
grep -q "via_cycles_sound" "$PORT_DIR/Tyu/Automation/Auto.lean" || { msg $RED "  G44 FAIL: via_cycles_sound missing (the composition theorem)"; g44_fail=1; }
grep -q "automation_rate" "$PORT_DIR/lakefile.toml" || { msg $RED "  G44 FAIL: automation_rate exe not registered"; g44_fail=1; }
grep -q "name = \"fill\"" "$PORT_DIR/lakefile.toml" || { msg $RED "  G44 FAIL: fill exe not registered"; g44_fail=1; }
grep -q "tyu:candidate obligation" crates/tyu/src/proof.rs || { msg $RED "  G44 FAIL: candidate markers not implemented in tyu proof fill"; g44_fail=1; }
grep -q "TYU_HARVEST_CANDIDATES" crates/tyu/src/proof.rs || { msg $RED "  G44 FAIL: harvest candidate attribution env missing"; g44_fail=1; }
grep -q 'authoredOf' "$PORT_DIR/Tyu/Verdicts/Harvest.lean" || { msg $RED "  G44 FAIL: harvest authoredOf missing"; g44_fail=1; }
grep -q "proven_no_candidates" crates/tyu/src/args.rs || { msg $RED "  G44 FAIL: --proven-no-candidates knob missing"; g44_fail=1; }
grep -q "render_candidate_ratios" crates/tyu/src/deploy.rs || { msg $RED "  G44 FAIL: deploy candidate-ratio rendering missing (§Q10)"; g44_fail=1; }
grep -q "candidates" crates/verifier/src/report.rs || { msg $RED "  G44 FAIL: report candidate field missing"; g44_fail=1; }
# FR-15: no JSON-crate codec in proof.rs (the gen-metadata reader is
# hand-rolled; `parse_gen_meta` + the attribution paths are all scanner code).
if grep -rn "serde" crates/tyu/src/proof.rs >/dev/null 2>&1; then
    msg $RED "  G44 FAIL: serde in proof.rs (FR-15)"
    grep -rn "serde" crates/tyu/src/proof.rs | head -3
    g44_fail=1
fi
# FR-14: the new codecs' digests are SHA-256 (candidates.json has no digests;
# the statement_hash already uses sha256). Guard the fnv boundary: the ONLY
# permitted fnv use in proof.rs is the pre-existing `cache::fnv1a_u64`
# cache-key helper (identity-key domain) — any other fnv reference in the
# attribution path fails.
if grep -rn "fnv" crates/tyu/src/proof.rs | grep -v "cache::fnv1a_u64" >/dev/null 2>&1; then
    msg $RED "  G44 FAIL: fnv outside the cache-key helper in proof.rs (FR-14 boundary)"
    grep -rn "fnv" crates/tyu/src/proof.rs | grep -v "cache::fnv1a_u64" | head -3
    g44_fail=1
fi
grep -q "P10 automation" ci/port.sh || { msg $RED "  G44 FAIL: the P10 gate is not wired into ci/port.sh"; g44_fail=1; }
grep -q "Tyu.Automation.via_cycles_sound" "$PORT_DIR/AxiomAudit.lean" || { msg $RED "  G44 FAIL: via_cycles_sound not axiom-audited"; g44_fail=1; }
[ "$g44_fail" -eq 0 ] && msg $GREEN "  G44: PLAN-VERIFY-3 P10 automation (Tyu.Automation library + exes + tyu proof fill + harvest attribution + --proven-no-candidates + axiom audit + port gate)"
failures=$((failures + g44_fail))

# G41: PLAN-VERIFY-3 P11.1/P11.3 — the automatic verify_manifest + the
# image-level deploy pairing (FR-8): the `tyu.vm/1` producer emits real
# (computed) hashes, and `tyu deploy` walks the import graph under a
# requiring policy (proven rejects unproven callees by name).
g41_fail=0
grep -q "write_module_summaries" crates/tyu/src/build.rs || { msg $RED "  G41 FAIL: build does not emit the per-module tyu.vm/1 summaries"; g41_fail=1; }
[ -f crates/tyu/src/vm_summary.rs ] || { msg $RED "  G41 FAIL: missing the tyu.vm/1 producer (vm_summary.rs)"; g41_fail=1; }
grep -q "statement_hash_hex" crates/tyu/src/vm_summary.rs || { msg $RED "  G41 FAIL: the summary must carry the canonical statement hashes"; g41_fail=1; }
grep -q "fn check_image_pairing" crates/tyu/src/deploy.rs || { msg $RED "  G41 FAIL: missing the image-level pairing gate"; g41_fail=1; }
grep -q "unproven callee" crates/tyu/src/deploy.rs || { msg $RED "  G41 FAIL: the pairing gate must reject unproven callees by name"; g41_fail=1; }
grep -q "resolve_graph" crates/tyu/src/deploy.rs || { msg $RED "  G41 FAIL: the pairing gate must walk the import graph"; g41_fail=1; }
[ -f crates/tyu/tests/vm_manifest.rs ] || { msg $RED "  G41 FAIL: missing the tyu.vm/1 producer test"; g41_fail=1; }
grep -q "proven_deploy_walks_the_import_graph" crates/tyu/tests/deploy_verify_policy.rs || { msg $RED "  G41 FAIL: missing the two-module pairing test"; g41_fail=1; }
[ "$g41_fail" -eq 0 ] && msg $GREEN "  G41: P11.1/P11.3 — automatic tyu.vm/1 producer + image-level deploy pairing (FR-8, compositional rule)"
failures=$((failures + g41_fail))

# G42: P10.2/P11 — the dead-CLI-form removal + the P11.1/P11.2 adjudications.
g42_fail=0
grep -q 'fn extract_artifacts_for_fill' crates/tyu/src/proof.rs || { msg $RED "  G42 FAIL: proof-fill standalone extraction missing"; g42_fail=1; }
grep -q '"--emit=obligations"' crates/tyu/src/proof.rs || { msg $RED "  G42 FAIL: the fill extraction must run langc --emit=obligations"; g42_fail=1; }
grep -q 'ProofArgs::Fill.*target' crates/tyu/src/args.rs || grep -q 'target,' crates/tyu/src/args.rs || { msg $RED "  G42 FAIL: proof fill --target not threaded"; g42_fail=1; }
grep -q 'RUNTIME_ABI_VERSION: u64 = 1' crates/lmod/src/abi_hash.rs || { msg $RED "  G42 FAIL: RUNTIME_ABI_VERSION drifted from 1"; g42_fail=1; }
grep -q 'PLAN-VERIFY-3 §13 adjudication' crates/lmod/src/abi_hash.rs || { msg $RED "  G42 FAIL: the ABI-bump adjudication record missing"; g42_fail=1; }
grep -q 'pub fn reference()' crates/hosted/src/loader.rs || { msg $RED "  G42 FAIL: the hosted reference policy (RequireNoOpen) missing"; g42_fail=1; }
grep -q 'fn verify_policy' crates/hosted/src/loader.rs || { msg $RED "  G42 FAIL: the hosted platform does not expose verify_policy"; g42_fail=1; }
[ -f crates/tyu/tests/proof_fill_e2e.rs ] || { msg $RED "  G42 FAIL: missing the standalone proof-fill e2e"; g42_fail=1; }
[ "$g42_fail" -eq 0 ] && msg $GREEN "  G42: P10.2/P11 — proof-fill standalone extraction live, RUNTIME_ABI adjudication recorded, hosted RequireNoOpen reference policy"
failures=$((failures + g42_fail))

# G43: cleanup — no drift between the two verify_manifest enforcers and no
# per-consumer copies of the summary→record pipeline; `--proven-no-candidates`
# must not be silently dropped by run/deploy.
g43_fail=0
grep -q "fn satisfies" crates/lmod/src/verify_manifest.rs || { msg $RED "  G43 FAIL: missing the shared satisfies()"; g43_fail=1; }
grep -q "satisfies" crates/loader-core/src/load.rs || { msg $RED "  G43 FAIL: the loader must use the shared satisfies()"; g43_fail=1; }
grep -q "satisfies" crates/tyu/src/deploy.rs || { msg $RED "  G43 FAIL: the deploy gate must use the shared satisfies()"; g43_fail=1; }
grep -q "fn encode_from_json_text" crates/lmod-pack/src/verify.rs || { msg $RED "  G43 FAIL: missing encode_from_json_text"; g43_fail=1; }
[ "$(grep -c 'encode_from_json_text' crates/tyu/src/build.rs crates/tyu/src/deploy.rs crates/lmod-pack/src/main.rs 2>/dev/null | awk -F: '{s+=$2} END{print s}')" -ge 3 ] || { msg $RED "  G43 FAIL: encode_from_json_text must be the single consumer path (build/deploy/lmod-pack)"; g43_fail=1; }
grep -q "proven_no_candidates: self.proven_no_candidates" crates/tyu/src/args.rs || { msg $RED "  G43 FAIL: deploy/run must forward --proven-no-candidates"; g43_fail=1; }
grep -q "fn render_candidate_ratios" crates/tyu/src/deploy.rs || { msg $RED "  G43 FAIL: missing the deploy candidate-ratio rendering (§Q10)"; g43_fail=1; }
[ "$g43_fail" -eq 0 ] && msg $GREEN "  G43: cleanup — shared satisfies() (loader+deploy), single encode_from_json_text path, --proven-no-candidates not dropped, §Q10 ratio rendered"
failures=$((failures + g43_fail))

# G45: PLAN-VERIFY-3 P11.3 — the certification-package surface (`tyu.cert`)
# The `<image>.tyucert/` package, its B4→B1→B2→B3 bindings, the `tyu cert
# verify|show|diff` CLI, the E6503 deploy FR-21 gate, the fuzz target, and
# the error-registry rows must all be present. Behaviour is exercised by the
# test suites (cert_assembly/cert_verify/deploy_pairing under
# `cargo test --workspace`, tools-gated where a real deploy is needed); this
# gate pins the *surface* in the regular Rust CI, like G40–G44.
g45_fail=0
if [ ! -f crates/tyu/src/cert.rs ]; then
    msg $RED "  G45 FAIL: missing the certification-package module (crates/tyu/src/cert.rs)"
    g45_fail=1
fi
grep -q 'code: E_CERT_PAIRING' crates/tyu/src/cert.rs || { msg $RED "  G45 FAIL: E6503 E_CERT_PAIRING missing"; g45_fail=1; }
for tok in 'assemble_for_deploy' 'verify_package' 'parse_index' 'pub fn show' 'pub fn diff' 'package.sig'; do
    if ! grep -q "$tok" crates/tyu/src/cert.rs; then
        msg $RED "  G45 FAIL: cert.rs lacks $tok"
        g45_fail=1
    fi
done
# CLI surface: `tyu cert verify|show|diff` parsed + dispatched.
grep -q 'Cert(CertArgs)' crates/tyu/src/args.rs || { msg $RED "  G45 FAIL: Command::Cert unparsed"; g45_fail=1; }
grep -q 'parse_cert' crates/tyu/src/args.rs || { msg $RED "  G45 FAIL: parse_cert missing"; g45_fail=1; }
grep -q 'Command::Cert(cert_args)' crates/tyu/src/main.rs || { msg $RED "  G45 FAIL: tyu cert not dispatched in main.rs"; g45_fail=1; }
# FR-21: the deploy wires the pre-ship B1/B2 verification (E6503).
grep -q 'assemble_for_deploy' crates/tyu/src/deploy.rs || { msg $RED "  G45 FAIL: deploy does not assemble the certification package"; g45_fail=1; }
# E6503 cert variant in the host error type.
grep -q 'Cert { code: u32' crates/tyu/src/error.rs || { msg $RED "  G45 FAIL: TyuError::Cert missing"; g45_fail=1; }
# Tests (hermetic always-run; deploy e2e is tools-gated inside).
for t in crates/tyu/tests/cert_assembly.rs crates/tyu/tests/cert_verify.rs crates/tyu/tests/deploy_pairing.rs; do
    if [ ! -f "$t" ]; then
        msg $RED "  G45 FAIL: $t missing"
        g45_fail=1
    fi
done
# Fuzz target registered + seeded.
if ! grep -q 'name = "cert_index_decode"' fuzz/Cargo.toml; then
    msg $RED "  G45 FAIL: cert_index_decode fuzz target not registered"
    g45_fail=1
fi
if [ ! -f fuzz/fuzz_targets/cert_index_decode.rs ]; then
    msg $RED "  G45 FAIL: cert_index_decode fuzz harness missing"
    g45_fail=1
fi
# Error-registry rows (65xx band).
for code in E6500 E6501 E6502 E6503 E6504 E6510; do
    if ! grep -q "$code" docs/book/appendix-b-error-registry.md; then
        msg $RED "  G45 FAIL: appendix-b error registry missing $code"
        g45_fail=1
    fi
done
[ "$g45_fail" -eq 0 ] && msg $GREEN "  G45: P11.3 certification-package surface (cert.rs + tyu cert CLI + FR-21 deploy gate + E6503/E6504 + fuzz + registry)"
failures=$((failures + g45_fail))

# G46: PLAN-VERIFY-3 P12.1 — platform.toml schema 3 [model], identity flow,
# and the lint matrix (§6.7, §Q15, FR-10).
# Pins the *surface*: the [model] manifest model, the pack-sourced model id
# accessors, the langc --model-semantics pass-through (with the P1.2
# hardcoding retired), the E5413–E5416 lint codes plus the warnings channel,
# the report's per-module (target, model) identity, and the build-side E6510
# (proven × unmodeled). Behaviour is exercised by platform_model_lint /
# model_id_flow / unmodeled_pipeline under `cargo test --workspace`.
g46_fail=0
grep -q 'MANIFEST_SCHEMA_MODEL: u32 = 3' crates/tyu/src/platform/config.rs || { msg $RED "  G46 FAIL: manifest schema-3 constant missing"; g46_fail=1; }
for tok in 'pub struct ModelSection' 'pub fn model_semantics' 'MMIO_VALUES' 'CONCURRENCY_VALUES'; do
    grep -q "$tok" crates/tyu/src/platform/config.rs || { msg $RED "  G46 FAIL: config.rs lacks $tok"; g46_fail=1; }
done
for code in E_PACK_MODEL_ARTIFACT_MISSING E_PACK_MODEL_UNDECLARED_ARTIFACT E_PACK_MODEL_ENUM_INVALID E_PACK_MODEL_EVIDENCE_MISSING; do
    grep -q "$code" crates/tyu/src/platform/lint.rs || { msg $RED "  G46 FAIL: lint.rs lacks $code"; g46_fail=1; }
done
grep -q 'pub warnings' crates/tyu/src/platform/lint.rs || { msg $RED "  G46 FAIL: LintOutcome lacks the warnings channel"; g46_fail=1; }
# langc: the flag parses, the drivers take the id, and the P1.2 unmodeled
# hardcoding is gone from the driver.
grep -q 'model-semantics=' crates/langc/src/args.rs || { msg $RED "  G46 FAIL: langc --model-semantics unparsed"; g46_fail=1; }
grep -q 'model_semantics: &\[u8\]' crates/langc/src/driver.rs || { msg $RED "  G46 FAIL: driver does not take the model id"; g46_fail=1; }
if grep -n 'MODEL_UNMODELED' crates/langc/src/driver.rs >/dev/null 2>&1; then
    msg $RED "  G46 FAIL: driver.rs still hardcodes MODEL_UNMODELED (P1.2 default must be retired)"
    g46_fail=1
fi
# tyu: the build forwards the pack-sourced id; the verdicts cache keys on it.
grep -q 'for_build(model_semantics)' crates/tyu/src/build.rs || { msg $RED "  G46 FAIL: build does not key VerifyEnvKey on the pack model"; g46_fail=1; }
grep -q -- '--model-semantics={}' crates/tyu/src/build.rs || { msg $RED "  G46 FAIL: build does not forward --model-semantics to langc"; g46_fail=1; }
# Report: per-module (target, model) identity (§7.3).
grep -q 'pub target: String' crates/verifier/src/report.rs || { msg $RED "  G46 FAIL: ModuleAccounting lacks target"; g46_fail=1; }
grep -q 'pub model: String' crates/verifier/src/report.rs || { msg $RED "  G46 FAIL: ModuleAccounting lacks model"; g46_fail=1; }
# Build-side E6510 (§Q15(c)/§11.9).
grep -q 'E_MODEL_UNMODELED' crates/tyu/src/verify.rs || { msg $RED "  G46 FAIL: build-side E6510 (proven × unmodeled) missing"; g46_fail=1; }
# The slice's test suites exist.
for t in platform_model_lint model_id_flow unmodeled_pipeline; do
    if [ ! -f "crates/tyu/tests/$t.rs" ] && [ ! -f "crates/tooling-tests/tests/$t.rs" ]; then
        msg $RED "  G46 FAIL: missing test suite $t"
        g46_fail=1
    fi
done
[ "$g46_fail" -eq 0 ] && msg $GREEN "  G46: P12.1 platform model semantics (schema 3 [model] + identity flow + lint matrix + E6510)"
failures=$((failures + g46_fail))

# G47: PLAN-VERIFY-3 P12.2 — bundle model instances (model artifacts +
# evidence corpora + Lean Tyu.Bundles instances + T-D subset theorems, §6.7
# / §Q16 / FR-11) and the Rust×Lean conformance pin.
# Pins the *surface*: the three modeled packs carry `[model]` ids matching
# their model artifacts; the evidence corpora exist and are committed;
# the Lean instance files + T-D registry theorems + audit lines exist; the
# bundle conformance test suite exists; the port gate runs the bundle
# corpora. Behaviour is exercised by bundle_instance_conformance /
# platform_model_lint / unmodeled_pipeline under `cargo test --workspace`
# and by `ci/port.sh lean`.
g47_fail=0
for triple in x86_64-unknown-none armv7m-unknown-none riscv32-unknown-none; do
    manifest="platforms/$triple/platform.toml"
    artifact="platforms/$triple/model/model.toml"
    corpus="platforms/$triple/evidence/vectors.json"
    grep -q "model_semantics = \"tyu.model/$triple/1\"" "$manifest" \
        || { msg $RED "  G47 FAIL: $manifest lacks the declared model id"; g47_fail=1; }
    [ -f "$artifact" ] \
        || { msg $RED "  G47 FAIL: missing model artifact $artifact"; g47_fail=1; }
    grep -q "\[memory\]" "$artifact" \
        || { msg $RED "  G47 FAIL: $artifact lacks [memory] geometry"; g47_fail=1; }
    grep -q "\[refinements\]" "$artifact" \
        || { msg $RED "  G47 FAIL: $artifact lacks the refinement manifest"; g47_fail=1; }
    [ -f "$corpus" ] \
        || { msg $RED "  G47 FAIL: missing evidence corpus $corpus"; g47_fail=1; }
    grep -q '"schema": "tyu.vec/1"' "$corpus" \
        || { msg $RED "  G47 FAIL: $corpus is not a tyu.vec/1 document"; g47_fail=1; }
    grep -q '"ram": \[' "$corpus" \
        || { msg $RED "  G47 FAIL: $corpus lacks the ram header"; g47_fail=1; }
done
# The Lean bundle instances + their registry theorems + audit lines.
for inst in Tyu/Bundles/Bundle.lean Tyu/Bundles/X86_64.lean Tyu/Bundles/ArmV7M.lean Tyu/Bundles/Riscv32.lean; do
    [ -f "verification/ports/lean/$inst" ] || { msg $RED "  G47 FAIL: missing $inst"; g47_fail=1; }
done
grep -q "import Tyu.Bundles" verification/ports/lean/Tyu/Sound.lean || { msg $RED "  G47 FAIL: Sound.lean does not import the bundle substrate"; g47_fail=1; }
for thm in Tyu.Sound.TD.store_load Tyu.Sound.TD.frame_law Tyu.Sound.TD.aperture_width_bound Tyu.Sound.TD.aperture_width_bound_full Tyu.Sound.TD.x86_64_geometry Tyu.Sound.TD.x86_64_inram Tyu.Sound.TD.armv7m_geometry Tyu.Sound.TD.armv7m_inram Tyu.Sound.TD.riscv32_geometry Tyu.Sound.TD.riscv32_inram Tyu.Sound.TD.bundle_aperture_agrees Tyu.Sound.TD.bundle_load_agrees; do
    grep -q "$thm" verification/ports/lean/AxiomAudit.lean || { msg $RED "  G47 FAIL: $thm not audited"; g47_fail=1; }
    grep -q "$thm" verification/ports/lean/REVIEW.md || { msg $RED "  G47 FAIL: $thm not in REVIEW.md §3"; g47_fail=1; }
done
# Conformance runner carries the bundle model + the file-fallback reader.
# (P14 consolidation: the single abstract model lives in `Tyu/Abs.lean` —
# the P12.2 `.bundle` constructor is the absorbed home; `Conformance/Step.lean`
# re-exports it by `abbrev`.)
grep -q "| bundle (ramLo" verification/ports/lean/Tyu/Abs.lean || { msg $RED "  G47 FAIL: the abstract model lacks the bundle constructor (Tyu/Abs.lean)"; g47_fail=1; }
grep -q "| bundle (ramLo" verification/ports/lean/Tyu/Conformance/Step.lean \
    && { msg $RED "  G47 FAIL: Conformance/Step.lean still carries a duplicated bundle model (P14 consolidation)"; g47_fail=1; }
grep -q "vectors.json" verification/ports/lean/Main.lean || { msg $RED "  G47 FAIL: conformance lacks the vectors.json fallback"; g47_fail=1; }
# Rust×Lean conformance suite + the port gate wiring.
[ -f "crates/verifier/tests/bundle_instance_conformance.rs" ] || { msg $RED "  G47 FAIL: missing bundle_instance_conformance"; g47_fail=1; }
grep -q "BUNDLE_CORPUS" ci/port.sh || { msg $RED "  G47 FAIL: ci/port.sh lacks the bundle corpus gate"; g47_fail=1; }
# P12 findings 4/5: the lint checks `evidence/vectors.json` *specifically*,
# the build path enforces the pairing gate, and the model-only stale case is
# unit-tested.
grep -q "evidence\").join(\"vectors.json\")\|evidence/vectors.json" crates/tyu/src/platform/lint.rs || { msg $RED "  G47 FAIL: lint does not require evidence/vectors.json specifically"; g47_fail=1; }
grep -q "pub fn ensure_model_pairing" crates/tyu/src/platform/lint.rs || { msg $RED "  G47 FAIL: missing the build-time ensure_model_pairing gate"; g47_fail=1; }
grep -q "ensure_model_pairing(selection.pack)\|ensure_model_pairing(&selection.pack)" crates/tyu/src/build.rs || { msg $RED "  G47 FAIL: build does not run the model pairing gate"; g47_fail=1; }
grep -q "stale_harvest_under_another_model_is_rejected" crates/tyu/src/vm_summary.rs || { msg $RED "  G47 FAIL: missing the model-only stale test"; g47_fail=1; }
[ "$g47_fail" -eq 0 ] && msg $GREEN "  G47: P12.2 bundle model instances (model artifacts + evidence corpora + Lean Tyu.Bundles + T-D registry + Rust×Lean conformance)"
failures=$((failures + g47_fail))

# --- G48: PLAN-VERIFY-3 P14 — the re-derivation surface (T-A/T-B) ---
# The P14 slice's contract in the regular Rust CI: the single interval
# implementation (`Tyu/Abs.lean`), the conformance re-export shims (no
# duplicated interval code), the T-A/T-B registry theorems, the `rederive`
# exe, the differential tests, and the gate script. Behavior is verified by
# `ci/port.sh` / `ci/differential.sh` (with the pinned toolchain); this gate
# pins the surface so a P14 regression cannot silently shed a piece.
g48_fail=0
PORT_DIR=verification/ports/lean
if [ ! -f "$PORT_DIR/Tyu/Abs.lean" ]; then
    msg $RED "  G48 FAIL: $PORT_DIR/Tyu/Abs.lean missing (the single interval implementation)"
    g48_fail=1
fi
# The consolidation: Conformance/Interval.lean + Step.lean are re-export shims
# (the duplicated defs are GONE — the implementation lives once in Tyu.Abs).
for shim in Tyu/Conformance/Interval.lean Tyu/Conformance/Step.lean; do
    if grep -q "def add : Interval\|def stepOp (op : OpInst)" "$PORT_DIR/$shim" 2>/dev/null; then
        msg $RED "  G48 FAIL: $shim still carries a duplicated interval definition (P14 consolidation)"
        g48_fail=1
    fi
done
# The registry theorems (present as theorems, proved + axiom-audited by the
# port gate).
for thm in "theorem add_sound" "theorem sub_sound" "theorem mul_sound" \
           "theorem tri_cmp_sound" "theorem tri_and_sound" "theorem tri_or_sound" \
           "theorem tri_not_sound" "theorem discharge_sound"; do
    if ! grep -q "$thm" "$PORT_DIR/Tyu/Sound.lean"; then
        msg $RED "  G48 FAIL: T-A/T-B theorem '$thm' missing from Tyu/Sound.lean"
        g48_fail=1
    fi
done
# The `rederive` exe + the differential test + the gate script.
if [ ! -f "$PORT_DIR/RederiveMain.lean" ] || ! grep -q '"rederive"' "$PORT_DIR/lakefile.toml"; then
    msg $RED "  G48 FAIL: the rederive exe surface missing (RederiveMain.lean / lakefile)"
    g48_fail=1
fi
if [ ! -f crates/verifier/tests/rederive_differential.rs ]; then
    msg $RED "  G48 FAIL: the re-derivation differential test missing (rederive_differential.rs)"
    g48_fail=1
fi
if [ ! -f ci/differential.sh ]; then
    msg $RED "  G48 FAIL: ci/differential.sh missing (the ≥10^5 differential gate)"
    g48_fail=1
fi
if [ ! -f crates/tooling-tests/tests/proven_automation_only.rs ]; then
    msg $RED "  G48 FAIL: the automation-only proven test missing (proven_automation_only.rs)"
    g48_fail=1
fi
if [ ! -f crates/verifier/test-vectors/rederive/index.json ]; then
    msg $RED "  G48 FAIL: the committed rederive corpus missing (test-vectors/rederive/index.json)"
    g48_fail=1
fi
[ "$g48_fail" -eq 0 ] && msg $GREEN "  G48: P14 re-derivation surface present (Tyu/Abs consolidation, T-A/T-B theorems, rederive exe, differential + tests)"
failures=$((failures + g48_fail))

# --- G49: PLAN-VERIFY-3 P15 — the services + concurrency-relativism surface ---
# The P15 slice's contract in the regular Rust CI: the abstract-atomic
# services model with its FIFO laws (axiom-audited via the port gate), the
# hosted bundle's model identity carrier + service corpus, the concurrency
# threading in the artifact/verdicts/report codecs, and the two e2e gates
# (the service-relativism tooling test + the hosted runtime leg).
g49_fail=0
if [ ! -f verification/ports/lean/Tyu/Services.lean ]; then
    msg $RED "  G49 FAIL: the abstract-atomic services model missing (Tyu/Services.lean)"
    g49_fail=1
fi
if [ ! -f sysroot/x86_64-unknown-linux-gnu/model.toml ]; then
    msg $RED "  G49 FAIL: the hosted bundle model identity carrier missing (sysroot/.../model.toml)"
    g49_fail=1
fi
if [ ! -f sysroot/x86_64-unknown-linux-gnu/evidence/vectors.json ]; then
    msg $RED "  G49 FAIL: the hosted bundle service-vector corpus missing (evidence/vectors.json)"
    g49_fail=1
fi
if [ ! -f crates/tooling-tests/tests/service_relativism.rs ]; then
    msg $RED "  G49 FAIL: the service-relativism gate test missing (service_relativism.rs)"
    g49_fail=1
fi
if [ ! -f crates/execution-tests/fixtures/conc_roundtrip.mod ]; then
    msg $RED "  G49 FAIL: the concurrency template fixture missing (conc_roundtrip.mod)"
    g49_fail=1
fi
for svc_fix in svc_fifo_send_recv svc_fifo_order svc_fifo_two_channels svc_fifo_deep_isolation; do
    if [ ! -f "crates/execution-tests/fixtures/${svc_fix}.mod" ]; then
        msg $RED "  G49 FAIL: the hosted wire fixture missing (${svc_fix}.mod)"
        g49_fail=1
    fi
done
if [ ! -f crates/verifier/src/svcvec.rs ]; then
    msg $RED "  G49 FAIL: the tyu.svcvec/1 codec missing (verifier/svcvec.rs)"
    g49_fail=1
fi
[ "$g49_fail" -eq 0 ] && msg $GREEN "  G49: P15 services surface present (Tyu.Services model, hosted identity + corpus, svcvec codec, wire fixtures, gate tests)"
failures=$((failures + g49_fail))

# G50: PLAN-VERIFY-3 P16.1/P16.2 — the QEMU anchor + the blame surface.
# The hardware anchor surfaces must exist as test binaries (proven_anchor,
# mutation_elision), the report's open[] intent/subject blame fields must be
# live (report.rs model + codec writer + verify.rs population + the E6410
# rendering), and the blame-surface rendering goldens test must exist.
g50_fail=0
for t in crates/execution-tests/tests/proven_anchor.rs crates/execution-tests/tests/mutation_elision.rs; do
    [ -f "$t" ] || { msg $RED "  G50 FAIL: $t missing (P16.1 anchor)"; g50_fail=1; }
done
for entry in proven_anchor mutation_elision; do
    grep -q "name *= *\"$entry\"" crates/execution-tests/Cargo.toml || { msg $RED "  G50 FAIL: execution-tests lacks [[test]] $entry"; g50_fail=1; }
done
grep -q "pub intent: Option<String>" crates/verifier/src/report.rs || { msg $RED "  G50 FAIL: OpenObligation.intent missing (blame surface)"; g50_fail=1; }
grep -q '\\"intent\\":' crates/verifier/src/codec.rs || { msg $RED "  G50 FAIL: report writer omits open[].intent"; g50_fail=1; }
grep -q "intent_of" crates/tyu/src/verify.rs || { msg $RED "  G50 FAIL: the closure intent lookup missing (verify.rs)"; g50_fail=1; }
grep -q "— intent:" crates/tyu/src/verify.rs || { msg $RED "  G50 FAIL: the E6410 intent rendering missing"; g50_fail=1; }
grep -q "(witness:" crates/tyu/src/verify.rs || { msg $RED "  G50 FAIL: the E6410 witness rendering missing"; g50_fail=1; }
[ -f crates/tooling-tests/tests/blame_surface.rs ] || { msg $RED "  G50 FAIL: blame_surface rendering-goldens test missing"; g50_fail=1; }
grep -q "read_declared_high" crates/execution-tests/tests/proven_anchor.rs || { msg $RED "  G50 FAIL: the anchor's measured ≤ declared channel missing"; g50_fail=1; }
[ "$g50_fail" -eq 0 ] && msg $GREEN "  G50: P16.1/P16.2 — QEMU anchor (proven_anchor/mutation_elision) + E6410 blame surface (witness + intent)"
failures=$((failures + g50_fail))

# G51: PLAN-VERIFY-3 P16.3 — acceptance matrix, fuzz closeout, NFR numbers.
# The acceptance runner, its mapping doc, the seeded fuzz smoke, the NFR
# measurer, and the tracked seeds (all three decode targets ≥ 1 seed each)
# must exist. Also pins the P16.3 fuzz finding (the codec livelock) as a
# regression test in the tree.
g51_fail=0
for f in ci/acceptance.sh ci/ACCEPTANCE.md ci/fuzz.sh ci/nfr.sh ci/fuzz-seeds; do
    [ -e "$f" ] || { msg $RED "  G51 FAIL: $f missing (P16.3 surface)"; g51_fail=1; }
done
for t in obl_v2_decode verdicts_v2_decode cert_index_decode; do
    n=$(find ci/fuzz-seeds/$t -type f 2>/dev/null | wc -l)
    if [ "$n" -lt 1 ]; then msg $RED "  G51 FAIL: no tracked seeds for $t"; g51_fail=1; fi
    grep -q "name = \"$t\"" fuzz/Cargo.toml || { msg $RED "  G51 FAIL: fuzz target $t unregistered"; g51_fail=1; }
    [ -f fuzz/fuzz_targets/$t.rs ] || { msg $RED "  G51 FAIL: fuzz harness $t.rs missing"; g51_fail=1; }
done
grep -q "invalid_utf8_lead_byte_in_string_fails_closed_not_hangs" crates/verifier/tests/codec_roundtrip.rs || { msg $RED "  G51 FAIL: the P16.3 codec livelock regression test missing"; g51_fail=1; }
grep -q 'len == 0 || start + len > self.b.len()' crates/verifier/src/codec.rs || { msg $RED "  G51 FAIL: the codec zero-width-sequence guard missing"; g51_fail=1; }
# The PLAN-VERIFY-2 superseded banner lives in the gitignored scratchpad
# (devdocs/, PLAN-RELEASE-1 §8 — its versioning is out of scope). It can
# never exist on a CI checkout, so the check is enforced when the scratchpad
# file is present locally and skips — never fails — when it is absent.
if [ -f devdocs/tyu-research/formal-verification.md ]; then
    grep -q "SUPERSEDED" devdocs/tyu-research/formal-verification.md || { msg $RED "  G51 FAIL: the PLAN-VERIFY-2 superseded banner missing"; g51_fail=1; }
else
    echo "  G51: PLAN-VERIFY-2 superseded banner: skipped (devdocs/tyu-research/formal-verification.md absent — gitignored scratchpad, no evidence to enforce)"
fi
[ "$g51_fail" -eq 0 ] && msg $GREEN "  G51: P16.3 — acceptance matrix (acceptance.sh + ACCEPTANCE.md), seeded fuzz smoke, NFR measurer, codec-livelock fix + regression"
failures=$((failures + g51_fail))

# G52: PLAN-RELEASE-1 S1 — the repo's own delivery path is evidence.
# `.github` MUST be tracked: a gitignored CI surface is an evidence-deletion
# anti-pattern for a fail-closed-verification toolchain, and it is exactly
# the state this release-engineering plan exists to retire. This guard makes
# it impossible for the `.gitignore` line to return (or the workflows to go
# missing) without a deliberate, recorded change.
#
# The complete shipped workflow surface (kept in one list; G52 and G53 share
# it so the "must exist" and "must timeout" invariants cannot drift apart).
TRACKED_WORKFLOWS=".github/workflows/ci.yml .github/workflows/integration.yml .github/workflows/install-verify.yml .github/workflows/release.yml"
g52_fail=0
if grep -qx '.github' .gitignore 2>/dev/null; then
    msg $RED "  G52 FAIL: .gitignore ignores .github — the CI evidence surface must stay tracked (PLAN-RELEASE-1 S1)"
    g52_fail=1
fi
for wf in $TRACKED_WORKFLOWS; do
    if [ ! -f "$wf" ]; then
        msg $RED "  G52 FAIL: $wf missing — the tracked workflow surface is incomplete (PLAN-RELEASE-1 S1)"
        g52_fail=1
    fi
done
[ "$g52_fail" -eq 0 ] && msg $GREEN "  G52: .github tracked — the CI evidence surface is repository truth"
failures=$((failures + g52_fail))

# G53: PLAN-RELEASE-1 S2 (plan's G32) — every workflow job declares
# `timeout-minutes`. An unbounded job makes a red diff unreadable and a hung
# job unkillable; the NFR-1 wall-clock budgets are only enforceable because
# every leg is bounded.
g53_fail=0
for wf in $TRACKED_WORKFLOWS; do
    [ -f "$wf" ] || continue
    jobs=$(awk '/^jobs:/{in_jobs=1; next} in_jobs && /^  [a-zA-Z0-9_-]+:/{c++} END{print c+0}' "$wf")
    timeouts=$(grep -c "timeout-minutes:" "$wf")
    if [ "$timeouts" -lt "$jobs" ]; then
        msg $RED "  G53 FAIL: $wf has $jobs jobs but only $timeouts timeout-minutes"
        g53_fail=1
    fi
done
[ "$g53_fail" -eq 0 ] && msg $GREEN "  G53: every workflow job carries timeout-minutes"
failures=$((failures + g53_fail))

# G54: PLAN-RELEASE-1 S3 (plan's G31) — the toolchain is a DATED nightly.
# An undated `channel = "nightly"` resolves to whichever nightly is latest at
# build time, which is exactly the reproducibility failure S3 exists to kill
# (three months of nightly moved the clippy bar and broke loader tests before
# the pin landed). The pin's format is enforced here; its buildability is the
# pin-audit CI job's job (FR-5).
g54_fail=0
pin="$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml | head -1)"
case "$pin" in
    nightly-[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]) ;;
    *)
        msg $RED "  G54 FAIL: rust-toolchain.toml channel must be a dated nightly (nightly-YYYY-MM-DD), got '${pin:-<unset>}'"
        g54_fail=1
        ;;
esac
[ "$g54_fail" -eq 0 ] && msg $GREEN "  G54: toolchain pin is a dated nightly ($pin)"
failures=$((failures + g54_fail))

# G55: PLAN-RELEASE-1 S9 (FR-7) — a v* tag ref must be the canonical
# three-component zero-patch form (`vMAJOR.MINOR.0`) with a changelog section.
# Enforced by delegating to ci/release-verify-tag.sh — the SINGLE
# implementation of the tag-format check (shared with release.yml's `verify`
# gate), so the regex cannot drift between the fail-fast gate and this
# repo-invariant. Non-tag refs (every branch/PR run, and local dev) are
# vacuously green — the invariant only bites when a v* tag ref is actually
# being processed (the release gate runs guards.sh on the tag ref).
g55_fail=0
g55_tag=""
case "${GITHUB_REF:-}" in
    refs/tags/*) g55_tag="${GITHUB_REF#refs/tags/}" ;;
    *)
        # local/branch run: enforce only when HEAD is exactly a v* tag
        g55_head="$(git describe --tags --exact-match HEAD 2>/dev/null || true)"
        case "$g55_head" in
            v*) g55_tag="$g55_head" ;;
        esac
        ;;
esac
if [ -n "$g55_tag" ]; then
    if bash ci/release-verify-tag.sh "$g55_tag" >/dev/null 2>&1; then
        msg $GREEN "  G55: $g55_tag is a canonical vMAJOR.MINOR.0 release tag (FR-7)"
    else
        msg $RED "  G55 FAIL: $g55_tag is not a canonical vMAJOR.MINOR.0 release tag (FR-7 / §Q2 — real patch tags are forbidden; the SHA is the patch identity)"
        g55_fail=1
    fi
else
    msg $GREEN "  G55: no release-tag ref — tag-form invariant vacuous"
fi
failures=$((failures + g55_fail))

echo ""
msg $GREEN "============================================"
msg $GREEN "Per-package test counts:"
echo "$counts" | tr '|' '\n' | sort | while IFS= read -r line; do
    case "$line" in
        *FAIL*)        msg $RED "$line" ;;
        *ALLOWED*)     msg $YELLOW "$line" ;;
        PKG*)          msg $GREEN "$line" ;;
    esac
done
echo ""
msg $GREEN "Total workspace tests (default features): $total_tests"

if [ "$failures" -gt 0 ]; then
    msg $RED "FAILED: $failures gate(s) failed — see the FAIL lines above"
    exit 1
fi
msg $GREEN "All guard checks passed."
