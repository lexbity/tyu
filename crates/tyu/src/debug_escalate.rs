//! A-side escalation debugger.
//!
//! When the in-guest B agent fails to emit a `D` diagnostic (NO_COMPLETION,
//! HANG, or timeout), this module re-runs the same image under QEMU with a
//! gdbstub, attaches via the RSP client, sets breakpoints on the trap
//! handlers, reads the register payload on hit, and constructs a
//! `DiagRecord` with `origin = 2` (gdbstub escalation).

use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use rsp_client::{connect_retry, regs, RspClient};

use crate::runner::{QemuDebugStart, Runner};
use codegen_core::Target;

// ---------------------------------------------------------------------------
// Per-target register table
// ---------------------------------------------------------------------------

/// Register numbers needed to read the `__lang_trap_loc` payload from a
/// stopped CPU via GDB's `p` (read register) command.
struct TargetRegs {
    pub trap_code: u8,
    pub valid: u8,
    pub source_line: u8,
    pub word_hash: u8,
    pub ds_ptr: u8,
    pub trap_pc: u8,
    /// slot_bytes for ds_depth = (ds_ptr - ds_base) / slot_bytes
    pub slot_bytes: u32,
}

/// Return GDB register numbers for the trap payload, per target.
fn register_table(target: Target) -> TargetRegs {
    use codegen_core::Target::*;
    match target {
        X86_64UnknownLinuxGnu | X86_64UnknownNone => TargetRegs {
            trap_code: regs::x86_64::RDI,
            valid: regs::x86_64::RSI,
            source_line: regs::x86_64::RDX,
            word_hash: regs::x86_64::RCX,
            ds_ptr: regs::x86_64::DS_PTR,
            trap_pc: regs::x86_64::RIP,
            slot_bytes: 8,
        },
        ArmV7MUnknownNone => TargetRegs {
            trap_code: regs::arm::TRAP_CODE,
            valid: regs::arm::VALID,
            source_line: regs::arm::LINE,
            word_hash: regs::arm::WORD_HASH_LO,
            ds_ptr: regs::arm::DS_PTR,
            trap_pc: regs::arm::PC,
            slot_bytes: 4,
        },
        RiscV32UnknownNone => TargetRegs {
            trap_code: regs::riscv::TRAP_CODE,
            valid: regs::riscv::VALID,
            source_line: regs::riscv::LINE,
            word_hash: regs::riscv::WORD_HASH_LO,
            ds_ptr: regs::riscv::DS_PTR,
            trap_pc: regs::riscv::PC,
            slot_bytes: 4,
        },
    }
}

// ---------------------------------------------------------------------------
// Helper: ephemeral port
// ---------------------------------------------------------------------------

/// Allocate an ephemeral loopback port by binding a listener to `:0`.
fn ephemeral_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind ephemeral port: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("get ephemeral port: {}", e))?
        .port();
    // Drop listener so QEMU can bind the port.
    drop(listener);
    Ok(port)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Outcome of the escalation attempt.
#[derive(Debug)]
pub struct EscalationOutcome {
    /// The decoded diagnostic, if escalation succeeded.
    pub diagnostic_string: Option<String>,
    /// Human-readable error message if escalation failed.
    pub error: Option<String>,
    /// Resolved target used for the escalation attempt.
    pub target: Target,
    /// Debug start mode used when launching QEMU.
    pub mode: QemuDebugStart,
    /// GDB stub port used for the escalation attempt.
    pub port: u16,
    /// Phase where the failure occurred, if any.
    pub phase: Option<&'static str>,
}

struct EscalationFailure {
    phase: &'static str,
    message: String,
}

/// Returns the symbolic address (from ELF `.symtab`) of a symbol by name,
/// using `nm`.
fn symbol_address(elf: &Path, sym_name: &str) -> Result<Option<u64>, String> {
    let out = Command::new("nm")
        .arg("--defined-only")
        .arg(elf)
        .output()
        .map_err(|e| format!("run nm on {}: {}", elf.display(), e))?;
    if !out.status.success() {
        return Err(format!(
            "nm --defined-only {} exited with {}",
            elf.display(),
            out.status
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && parts[2] == sym_name {
            return u64::from_str_radix(parts[0], 16)
                .map(Some)
                .map_err(|e| format!("parse nm output for {}: {}", sym_name, e));
        }
    }
    Ok(None)
}

/// Helper to convert gdb register bytes (up to 8 LE) to u64.
fn read_reg64(client: &mut RspClient, reg: u8) -> Result<u64, String> {
    let raw = client
        .read_register(reg)
        .map_err(|e| format!("read reg {}: {}", reg, e))?;
    let mut arr = [0u8; 8];
    let len = raw.len().min(8);
    arr[..len].copy_from_slice(&raw[..len]);
    Ok(u64::from_le_bytes(arr))
}

fn connect_rsp_client(
    qemu: &mut std::process::Child,
    port: u16,
    phase: &'static str,
) -> Result<RspClient, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = qemu
            .try_wait()
            .map_err(|e| format!("{}: wait QEMU: {}", phase, e))?
        {
            return Err(format!(
                "{}: QEMU exited before RSP connect ({})",
                phase, status
            ));
        }
        match connect_retry("127.0.0.1", port) {
            Ok(stream) => {
                return RspClient::from_stream(stream).map_err(|e| format!("{}: {}", phase, e))
            }
            Err(e) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
                if e.kind() == std::io::ErrorKind::ConnectionRefused
                    || e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::ConnectionReset
                    || e.kind() == std::io::ErrorKind::WouldBlock
                {
                    continue;
                }
                continue;
            }
            Err(e) => return Err(format!("{}: {}", phase, e)),
        }
    }
}

fn sample_running_pc(client: &mut RspClient, regs: &TargetRegs) -> Result<u64, String> {
    match read_reg64(client, regs.trap_pc) {
        Ok(pc) => Ok(pc),
        Err(first) => {
            client
                .interrupt()
                .map_err(|e| format!("interrupt target before PC sample: {}", e))?;
            read_reg64(client, regs.trap_pc).map_err(|second| {
                format!(
                    "read PC after interrupt failed: {}; initial read failed: {}",
                    second, first
                )
            })
        }
    }
}

/// Main escalation entry point.
///
/// Launches QEMU with gdbstub, attaches, reads the trap state, and returns
/// a formatted diagnostic string.
pub fn escalate(
    image: &Path,
    target: Target,
    source_map: Option<(&Path, &diag_core::render::SourceMap)>,
) -> EscalationOutcome {
    let mode = QemuDebugStart::FrozenAtReset;
    let spec = match target.spec().qemu {
        Some(s) => s,
        None => {
            return EscalationOutcome {
                diagnostic_string: None,
                error: Some("escalation requires a QEMU target".into()),
                target,
                mode,
                port: 0,
                phase: Some("target"),
            };
        }
    };

    let regs = register_table(target);
    let port = match ephemeral_port() {
        Ok(p) => p,
        Err(e) => {
            return EscalationOutcome {
                diagnostic_string: None,
                error: Some(format!("escalation: ephemeral port: {}", e)),
                target,
                mode,
                port: 0,
                phase: Some("port"),
            };
        }
    };

    // 1. Resolve trap entry symbols from the ELF. Codegen routes runtime
    //    checks to one of several entry points: most range/channel/subtype
    //    checks `jmp __lang_trap` (generic, no source payload), loc-bearing
    //    traps use `__lang_trap_loc`, and data-stack overflow uses
    //    `__stack_overflow`. We break at every entry that exists and
    //    disambiguate by PC after the stop. Breaking only `__lang_trap_loc`
    //    (the previous behavior) missed the common generic-trap path entirely,
    //    so `continue` never stopped.
    macro_rules! resolve {
        ($name:expr) => {
            match symbol_address(image, $name) {
                Ok(a) => a,
                Err(e) => {
                    return EscalationOutcome {
                        diagnostic_string: None,
                        error: Some(format!("escalation: symbol lookup failed: {}", e)),
                        target,
                        mode,
                        port,
                        phase: Some("symbol"),
                    };
                }
            }
        };
    }
    let trap_loc_addr = resolve!("__lang_trap_loc");
    let trap_addr = resolve!("__lang_trap");
    let overflow_addr = resolve!("__stack_overflow");
    let ds_base_addr = resolve!("__lang_ds_base").unwrap_or(0);

    // ARM Thumb function symbols carry bit 0 as a marker, but the gdbstub
    // breakpoint must target the even instruction address. x86/RISC-V symbol
    // addresses are literal and may legitimately be odd (e.g. x86 `__lang_trap`
    // at 0x...e5), so only strip the bit on ARM.
    let strip_thumb = matches!(target, Target::ArmV7MUnknownNone);
    let norm = |a: u64| if strip_thumb { a & !1 } else { a };
    let breakpoints: Vec<u64> = [trap_loc_addr, trap_addr, overflow_addr]
        .into_iter()
        .flatten()
        .map(|a| norm(a))
        .collect();
    if breakpoints.is_empty() {
        return EscalationOutcome {
            diagnostic_string: None,
            error: Some(
                "escalation: no trap entry symbol (__lang_trap_loc/__lang_trap/__stack_overflow) \
                 found in ELF"
                    .into(),
            ),
            target,
            mode,
            port,
            phase: Some("symbol"),
        };
    }

    eprintln!(
        "escalation: target={:?} port={} breakpoints={:#x?} ds_base={:#x}",
        target, port, breakpoints, ds_base_addr,
    );

    // 2. Spawn QEMU with gdbstub.
    let mut qemu = match Runner::spawn_debug(spec, image, port, mode) {
        Ok(c) => c,
        Err(e) => {
            return EscalationOutcome {
                diagnostic_string: None,
                error: Some(format!("escalation: failed to spawn QEMU: {}", e)),
                target,
                mode,
                port,
                phase: Some("spawn"),
            };
        }
    };

    // 3. Connect RSP client.
    let result = (|| -> Result<String, EscalationFailure> {
        let mut client =
            connect_rsp_client(&mut qemu, port, "escalation connect").map_err(|e| {
                EscalationFailure {
                    phase: "connect",
                    message: e,
                }
            })?;

        // 4. Set a breakpoint at every resolved trap entry point.
        for &addr in &breakpoints {
            client.set_breakpoint(addr).map_err(|e| EscalationFailure {
                phase: "breakpoint",
                message: format!("set breakpoint at {:#x}: {}", addr, e),
            })?;
        }

        // 5. Continue execution. A breakpoint fires when any trap occurs.
        client.continue_exec().map_err(|e| EscalationFailure {
            phase: "continue",
            message: format!("continue: {}", e),
        })?;

        // 6. Identify which trap entry fired (compare PC, ignoring the Thumb
        //    bit), then read the appropriate payload. Only `__lang_trap_loc`
        //    carries a source payload; the generic `__lang_trap` and
        //    `__stack_overflow` paths report trap_code only (valid=0).
        let trap_pc = read_reg64(&mut client, regs.trap_pc).map_err(|e| EscalationFailure {
            phase: "trap-read",
            message: e,
        })?;
        let ds_ptr = read_reg64(&mut client, regs.ds_ptr).map_err(|e| EscalationFailure {
            phase: "trap-read",
            message: e,
        })?;
        let at = |addr: Option<u64>| addr.is_some_and(|a| norm(a) == norm(trap_pc));

        let read_code = |client: &mut RspClient| -> Result<u16, EscalationFailure> {
            Ok(
                read_reg64(client, regs.trap_code).map_err(|e| EscalationFailure {
                    phase: "trap-read",
                    message: e,
                })? as u16,
            )
        };

        let (trap_code, valid, source_line, word_hash) = if at(trap_loc_addr) {
            let trap_code = read_code(&mut client)?;
            let valid = read_reg64(&mut client, regs.valid).map_err(|e| EscalationFailure {
                phase: "trap-read",
                message: e,
            })? != 0;
            let source_line =
                read_reg64(&mut client, regs.source_line).map_err(|e| EscalationFailure {
                    phase: "trap-read",
                    message: e,
                })? as u32;
            let word_hash =
                read_reg64(&mut client, regs.word_hash).map_err(|e| EscalationFailure {
                    phase: "trap-read",
                    message: e,
                })?;
            (trap_code, valid, source_line, word_hash)
        } else if at(overflow_addr) {
            // Data-stack overflow: trap_code is implicit (10), no payload.
            (10u16, false, 0u32, 0u64)
        } else {
            // Generic `__lang_trap`: trap_code in the contract register, no payload.
            (read_code(&mut client)?, false, 0u32, 0u64)
        };

        // Compute ds_depth = (ds_ptr - ds_base) / slot_bytes.
        let ds_depth = if ds_base_addr != 0 && ds_ptr >= ds_base_addr {
            ((ds_ptr - ds_base_addr) / regs.slot_bytes as u64) as u32
        } else {
            0
        };

        // 7. Build a DiagRecord.
        let record = diag_core::DiagRecord {
            version: diag_core::DIAG_RECORD_VERSION,
            origin: diag_core::origin::GDBSTUB,
            valid,
            trap_code,
            source_line,
            word_hash,
            trap_pc,
            ds_depth,
            ds_declared: diag_core::DS_DECLARED_UNKNOWN,
            slot_count: 0,
        };

        // 8. Resolve and render.
        let elf_data = std::fs::read(image).unwrap_or_default();
        let modinfo_bytes =
            crate::test_cmd::read_elf_section_by_name_internal(&elf_data, b".lang.modinfo");
        let debug_bytes =
            crate::test_cmd::read_elf_section_by_name_internal(&elf_data, b".lang.debug");

        let index = if let Some(ref dbg) = debug_bytes {
            diag_core::decode::ModinfoIndex::from_debug_bytes(dbg)
        } else if let Some(ref mi) = modinfo_bytes {
            diag_core::decode::ModinfoIndex::from_modinfo_bytes(mi)
        } else {
            None
        };

        let diagnostic = match index {
            Some(ref idx) => diag_core::decode::resolve(&record, idx),
            None => diag_core::decode::Diagnostic {
                word_name: None,
                trap_code,
                claim_text: diag_core::claims::claim_text(trap_code),
                valid,
                origin: diag_core::origin::GDBSTUB,
                source_line,
                ds_depth,
                ds_declared: diag_core::DS_DECLARED_UNKNOWN,
            },
        };

        let empty_map = diag_core::render::SourceMap::new();
        let sm = source_map.map(|(_, m)| m).unwrap_or(&empty_map);
        let rendered = diagnostic.render(source_map.map(|(p, _)| p), sm);
        Ok(rendered)
    })();

    // 9. Clean up QEMU.
    let _ = qemu.kill();
    let _ = qemu.wait();

    match result {
        Ok(diag) => EscalationOutcome {
            diagnostic_string: Some(diag),
            error: None,
            target,
            mode,
            port,
            phase: None,
        },
        Err(failure) => EscalationOutcome {
            diagnostic_string: None,
            error: Some(format!(
                "escalation failed [{}]: {}",
                failure.phase, failure.message
            )),
            target,
            mode,
            port,
            phase: Some(failure.phase),
        },
    }
}

// ---------------------------------------------------------------------------
// Hang classification
// ---------------------------------------------------------------------------

/// Result of classifying a timed-out (hanging) image.
#[derive(Clone, Debug)]
pub enum HangClass {
    /// Infinite net-zero loop with finite stack (expected).
    PollLoop {
        /// Name of the word that was running.
        word: String,
    },
    /// Non-tail recursion causing unbounded stack growth (bug).
    RunawayRecursion {
        /// Name of the word that was running.
        word: String,
    },
    /// Could not determine the cause.
    Unknown {
        /// Word name at the sampled PC, if available.
        word: Option<String>,
        /// Reason classification failed.
        reason: String,
    },
}

impl std::fmt::Display for HangClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HangClass::PollLoop { word } => {
                write!(
                    f,
                    "hang classification: '{}' → poll loop (expected divergence, finite stack)",
                    word,
                )
            }
            HangClass::RunawayRecursion { word } => {
                write!(
                    f,
                    "hang classification: '{}' → runaway recursion suspected (⊤ stack bound)",
                    word,
                )
            }
            HangClass::Unknown { word, reason } => {
                write!(f, "hang classification: ")?;
                if let Some(w) = word {
                    write!(f, "'{}' → ", w)?;
                }
                write!(f, "unclassified: {}", reason)
            }
        }
    }
}

/// Classify a hanging image, using the default gdbstub port (1235).
pub fn classify_hang(image: &Path, target: Target) -> HangClass {
    classify_hang_on_port(image, target, 1235)
}

/// Classify a hanging image by connecting to QEMU's gdbstub, sampling the
/// program counter, and looking up the running word's static properties
/// from `.lang.debug`.
///
/// Launches QEMU in running mode so execution begins immediately, waits for
/// the hang to be reached, then samples PC via RSP.
///
/// `port` is the TCP port for the gdbstub.  Use different ports for
/// concurrent tests to avoid conflicts.
pub fn classify_hang_on_port(image: &Path, target: Target, port: u16) -> HangClass {
    let spec = match target.spec().qemu {
        Some(s) => s,
        None => {
            return HangClass::Unknown {
                word: None,
                reason: "not a QEMU target".into(),
            };
        }
    };

    // 1. Launch QEMU without -S so execution begins immediately.
    let mut qemu = match Runner::spawn_debug(spec, image, port, QemuDebugStart::RunImmediately) {
        Ok(c) => c,
        Err(e) => {
            return HangClass::Unknown {
                word: None,
                reason: format!("spawn QEMU: {}", e),
            };
        }
    };

    let result = (|| -> Result<HangClass, String> {
        // Let the guest run for a short interval so we sample a live PC, not
        // reset state. If the CPU still refuses a register read, stop it via
        // RSP interrupt and retry the sample.
        std::thread::sleep(Duration::from_millis(1500));
        let mut client = connect_rsp_client(&mut qemu, port, "hang connect")?;

        // 2. Sample the program counter using target-specific PC register.
        let regs = register_table(target);
        let pc = sample_running_pc(&mut client, &regs)?;

        // 3. Map PC → word name via nm -n (sorted symbols).
        let word_name = symbol_at_pc(image, pc);

        // 4. Look up the word in .lang.debug to get effects and high.
        let dw_info = word_name
            .as_deref()
            .and_then(|name| lookup_in_debugsec(image, name));

        // 5. Classify.
        match dw_info {
            Some(info) => Ok(classify_debug_word(info)),
            None => {
                if word_name.is_none() {
                    if let Some(info) = unique_unbounded_debug_word(image) {
                        return Ok(classify_debug_word(info));
                    }
                }
                let reason = if word_name.is_some() {
                    "word not found in .lang.debug".into()
                } else {
                    "could not map PC to any symbol".into()
                };
                Ok(HangClass::Unknown {
                    word: word_name,
                    reason,
                })
            }
        }
    })();

    // 6. Clean up.
    let _ = qemu.kill();
    let _ = qemu.wait();

    match result {
        Ok(hc) => hc,
        Err(e) => HangClass::Unknown {
            word: None,
            reason: e,
        },
    }
}

fn classify_debug_word(info: DebugWordInfo) -> HangClass {
    let diverge = info.effects & 0x04 != 0; // bit 2 = DIVERGE
    let is_top = info.high == 0xFFFF_FFFF;

    match (diverge, is_top) {
        (true, false) => HangClass::PollLoop { word: info.name },
        (true, true) => HangClass::RunawayRecursion { word: info.name },
        (false, true) => HangClass::RunawayRecursion { word: info.name },
        (false, false) => HangClass::Unknown {
            word: Some(info.name),
            reason: format!(
                "word does not diverge (effects={:#x}) and has finite high={}",
                info.effects, info.high,
            ),
        },
    }
}

/// Information about a word from `.lang.debug`.
struct DebugWordInfo {
    name: String,
    effects: u16,
    high: u32,
}

/// Look up a word by name in the `.lang.debug` section of an ELF.
fn lookup_in_debugsec(elf: &Path, name: &str) -> Option<DebugWordInfo> {
    let elf_data = std::fs::read(elf).ok()?;
    let debug_bytes =
        crate::test_cmd::read_elf_section_by_name_internal(&elf_data, b".lang.debug")?;

    // Derive the sym_hash key. `symbol_at_pc` returns the mangled linker
    // symbol `w_<16-hex of fnv1a_u64(word_name)>` (see codegen ophelpers),
    // and `.lang.debug` keys entries by that same fnv1a_u64(word_name). So the
    // hex suffix already *is* the sym_hash — parse it rather than re-hashing
    // the mangled string (which never matches). Fall back to hashing the name
    // directly if it is not a `w_`-mangled symbol.
    let hash = name
        .strip_prefix("w_")
        .filter(|hex| hex.len() == 16)
        .and_then(|hex| u64::from_str_radix(hex, 16).ok())
        .unwrap_or_else(|| fnv1a_str(name));

    // Iterate over debug entries to find the matching hash.
    let (count, _) = lmod::debugsec::decode_header(&debug_bytes)?;
    for i in 0..count {
        let entry = lmod::debugsec::read_entry(&debug_bytes, i)?;
        if entry.sym_hash == hash {
            return Some(DebugWordInfo {
                name: core::str::from_utf8(entry.name).ok()?.to_string(),
                effects: entry.effects,
                high: entry.high,
            });
        }
    }
    None
}

fn unique_unbounded_debug_word(elf: &Path) -> Option<DebugWordInfo> {
    let elf_data = std::fs::read(elf).ok()?;
    let debug_bytes =
        crate::test_cmd::read_elf_section_by_name_internal(&elf_data, b".lang.debug")?;
    let (count, _) = lmod::debugsec::decode_header(&debug_bytes)?;

    let mut candidate = None;
    let mut non_main_candidate = None;
    for i in 0..count {
        let entry = lmod::debugsec::read_entry(&debug_bytes, i)?;
        if entry.high != 0xFFFF_FFFF {
            continue;
        }
        let info = DebugWordInfo {
            name: core::str::from_utf8(entry.name).ok()?.to_string(),
            effects: entry.effects,
            high: entry.high,
        };
        if info.name != "main" {
            if non_main_candidate.is_some() {
                return None;
            }
            non_main_candidate = Some(DebugWordInfo {
                name: info.name.clone(),
                effects: info.effects,
                high: info.high,
            });
        }
        if candidate.is_some() {
            candidate = None;
        } else {
            candidate = Some(info);
        }
    }
    non_main_candidate.or(candidate)
}

/// FNV-1a 64-bit hash for a string slice.
fn fnv1a_str(s: &str) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

/// Find the symbol whose address range contains `pc`, using `nm -n`
/// (sorted by address).
fn symbol_at_pc(elf: &Path, pc: u64) -> Option<String> {
    let out = Command::new("nm")
        .args(["-n", &elf.to_string_lossy()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);

    let mut prev_name: Option<String> = None;
    let mut prev_addr: u64 = 0;

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let addr = u64::from_str_radix(parts[0], 16).ok()?;
        let name = parts[2];

        // Symbol type 't' or 'T' = code (function).
        // Only consider code symbols.
        if parts[1] != "t" && parts[1] != "T" {
            prev_name = None;
            prev_addr = addr;
            continue;
        }

        if pc >= prev_addr && pc < addr {
            return prev_name;
        }
        prev_name = Some(name.to_string());
        prev_addr = addr;
    }

    // Check the last symbol.
    if pc >= prev_addr {
        return prev_name;
    }

    None
}
