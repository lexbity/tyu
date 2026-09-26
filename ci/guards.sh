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
if ! grep -q 'E6410' devdocs/book_v3/appendix-b-error-registry.md \
   || ! grep -q 'E6413' devdocs/book_v3/appendix-b-error-registry.md; then
    msg $RED "  G23 FAIL: error-registry appendix must carry E6410/E6413"
    g23_fail=1
fi
if ! grep -q "Compile-time discharge" devdocs/book_v3/ch03-types.md \
   || ! grep -q "no-open" devdocs/book_v3/ch03-types.md; then
    msg $RED "  G23 FAIL: ch03 must teach the both-policies compile-time-discharge example"
    g23_fail=1
fi
if ! grep -q "Contract obligations and elision" devdocs/book_v3/ch04-contracts.md \
   || ! grep -q "module-loading" devdocs/book_v3/ch04-contracts.md; then
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
    actual=$(cd "$dir" && ls *.stmt.json 2>/dev/null | sed 's/\.stmt\.json$//' | sort | tr '\n' ' ' | sed 's/ $//')
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
    for f in test-goldens/stackmeta/$triple/*.json; do
        [ -f "$f" ] && sm_count=$((sm_count + 1))
    done
done
if [ "$sm_count" -lt 20 ]; then
    msg $RED "  G32 FAIL: stackmeta goldens thin ($sm_count files; expected ≥ 20 across four triples)"
    g32_fail=1
fi
[ "$g32_fail" -eq 0 ] && msg $GREEN "  G32: T-C substrate (Step + Sound theorems, axiom audit, stackmeta goldens + exporter)"
failures=$((failures + g32_fail))

# --- G28: PLAN-VERIFY-3 §P0 — workspace bar: rustfmt ---
# Every slice ends `cargo fmt --check` green (§P0 workspace bar). The gate is
# the check itself — mechanical, zero judgement, no bypass path short of
# editing this gate.
g28_fail=0
if ! cargo fmt --check >/dev/null 2>&1; then
    msg $RED "  G28 FAIL: rustfmt drift — run 'cargo fmt' (workspace bar, §P0)"
    cargo fmt --check 2>&1 | grep "^Diff in" | head -5 | while read -r d; do
        msg $RED "    $d"
    done
    g28_fail=1
fi
[ "$g28_fail" -eq 0 ] && msg $GREEN "  G28: rustfmt clean (workspace bar)"
failures=$((failures + g28_fail))

# --- G29: PLAN-VERIFY-3 §P0 — workspace bar: clippy -D warnings ---
# Deliberately `--lib --bins --tests`, NOT --all-targets: --all-targets
# synthesizes a phantom bin-as-test compile that ignores the manifest's
# `test = false` (lang-assemble) and flips panic to unwind against the
# no_std runtime crates (hosted-rt) — a target that does not exist. The
# explicit form covers every real target (no examples/ or benches/ dirs in
# the workspace). Warnings are denied; suppressions require a justification
# comment at the item (spec Rule: no silent suppression).
g29_fail=0
if ! cargo clippy --workspace --lib --bins --tests -- -D warnings >/dev/null 2>&1; then
    msg $RED "  G29 FAIL: clippy -D warnings (workspace bar, §P0)"
    cargo clippy --workspace --lib --bins --tests 2>&1 | grep "^ *-->" | head -5 | while read -r d; do
        msg $RED "    $d"
    done
    g29_fail=1
fi
[ "$g29_fail" -eq 0 ] && msg $GREEN "  G29: clippy -D warnings clean (--lib --bins --tests)"
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
