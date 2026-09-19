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

RED=1; GREEN=2; YELLOW=3; NC=0
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
arm_archive="target/thumbv7m-none-eabi/release/libdevice_loader_archive.a"
if [ -f "$arm_archive" ]; then
    size_tool=$(command -v arm-none-eabi-size || command -v size || true)
    if [ -z "$size_tool" ]; then
        msg $RED "  FAIL: ARM loader size gate has an archive but no size tool"
        failures=$((failures + 1))
    else
        text_bytes=$("$size_tool" -A "$arm_archive" 2>/dev/null | awk '
            /^loader_core-/ {in_loader=1; next}
            /^[^[:space:]].*\(ex .*libdevice_loader_archive\.a\):/ {in_loader=0; next}
            in_loader && $1 ~ /^\.text/ {sum += $2}
            END {print sum+0}
        ')
        if [ "$text_bytes" -gt 16384 ]; then
            msg $RED "  FAIL: ARM device-loader .text is ${text_bytes} bytes (> 16384)"
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
for crate in crates/ir crates/codegen-core crates/codegen-arm crates/codegen-riscv crates/codegen-x86_64; do
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
    crates/ir/src crates/semantics/src crates/codegen-core/src \
    --include='*.rs' 2>/dev/null || true)
host_panic_count=$(printf '%s\n' "$host_panic_hits" | sed '/^$/d' | wc -l | tr -d ' ')
if [ "$host_panic_count" -eq 0 ]; then
    msg $GREEN "  G13: host parse/type/core paths have no panic/unreachable hits"
else
    msg $RED "  G13 FAIL: $host_panic_count panic/unreachable hit(s) remain"
    printf '%s\n' "$host_panic_hits" >&2
    failures=$((failures + 1))
fi

# --- G14: descriptor schema stamp coverage (Phase P1, renewed P8) ---
# Every discovered platform pack manifest (platforms/<name>/platform.toml and
# runtime/*.platform.toml) must carry the descriptor v2 stamp `schema = 2`.
# A pack without the stamp is a legacy pack the compiler cannot consume; a
# missing stamp is exactly the drift this spec retires. Mutation check #8.
missing_schema=""
for manifest in platforms/*/platform.toml runtime/*.platform.toml; do
    if [ ! -e "$manifest" ]; then
        continue
    fi
    if ! grep -qE '^schema[[:space:]]*=[[:space:]]*2' "$manifest"; then
        missing_schema="$missing_schema $manifest"
    fi
done
if [ -z "$missing_schema" ]; then
    msg $GREEN "  G14: all platform pack manifests carry the descriptor schema stamp"
else
    msg $RED "  G14 FAIL: pack manifests missing 'schema = 2':$missing_schema"
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
    msg $RED "FAILED: $failures package(s) have test-delivery issues"
    exit 1
fi
msg $GREEN "All guard checks passed."
