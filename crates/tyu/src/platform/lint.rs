//! Platform-pack linting and build-time interface checks.

use crate::error::TyuError;
use codegen_core::Target;
use lmod::abi_hash::{compute_abi_hash, RUNTIME_ABI_VERSION};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use super::config::{
    discover_platforms_in, find_pack_manifest_path, load_platform_pack_from_text, CapabilityConfig,
    MemorySection, MetalSection, PlatformManifest, PlatformPack, TestRung,
};

const MAX_PACK_FILE_BYTES: u64 = 64 * 1024;
const REQUIRED_BASE_SYMBOLS: &[&str] = &[
    "__lang_start",
    "__lang_trap",
    "__lang_ds_base",
    "__lang_ds_limit",
    "__lang_ds_high",
    "__lang_expected_abi_hash",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintError {
    pub code: u16,
    pub detail: String,
}

impl LintError {
    fn new(code: u16, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintOutcome {
    pub pack: String,
    pub errors: Vec<LintError>,
}

const E_PACK_MANIFEST_INVALID: u16 = 5400;
const E_PACK_INTERFACE_MISMATCH: u16 = 5401;
const E_PACK_SYMBOL_MISSING: u16 = 5402;
const E_PACK_SECTION_MISSING: u16 = 5403;
const E_PACK_FEATURE_UNIT_MISSING: u16 = 5404;
const E_PACK_CAPABILITY_GLUE_MISSING: u16 = 5405;
const E_PACK_ABI_HASH_MISMATCH: u16 = 5406;
const E_PACK_TESTRUNG_UNBACKED: u16 = 5407;
const E_PACK_DEPLOY_RECIPE_INVALID: u16 = 5408;
const E_PACK_PATH_INVALID: u16 = 5410;
const E_PACK_FILE_TOO_LARGE: u16 = 5411;
const E_PACK_DEBUG_AGENT_UNBACKED: u16 = 5412;

pub fn lint_pack(root: &Path, name: &str, all: bool) -> Result<LintOutcome, TyuError> {
    let manifest_path = find_pack_manifest_path(root, name)
        .ok_or_else(|| TyuError::Platform(format!("platform pack '{}' not found", name)))?;
    let text = fs::read_to_string(&manifest_path)
        .map_err(|e| TyuError::Platform(format!("reading '{}': {}", manifest_path.display(), e)))?;
    let pack = match load_platform_pack_from_text(root, &manifest_path, &text) {
        Ok(pack) => pack,
        Err(e) => {
            return Ok(LintOutcome {
                pack: name.to_string(),
                errors: vec![LintError::new(E_PACK_MANIFEST_INVALID, e.to_string())],
            });
        }
    };
    let mut outcome = lint_pack_manifest(root, &pack, all)?;

    // Descriptor v2 validation (§5.2). A legacy pack (no v2 sections) adds
    // nothing; a v2 pack contributes E3646/E3647 errors to the outcome so
    // `tyu platform lint` fails on an invalid descriptor, exactly as it fails
    // on an invalid pack structure. The pack root backs the `metal.trust`
    // words-must-exist rule.
    match super::desc::parse::parse_descriptor(&text) {
        Ok(Some(desc)) => {
            for err in super::desc::validate::validate(&desc, Some(pack.pack_root())) {
                outcome.errors.push(LintError {
                    code: err.code,
                    detail: err.detail,
                });
            }
        }
        Ok(None) => {}
        Err(e) => {
            outcome.errors.push(LintError {
                code: e.code,
                detail: e.detail,
            });
        }
    }

    Ok(outcome)
}

pub fn ensure_build_platform_interface(root: &Path, target: Target) -> Result<(), TyuError> {
    let triple = std::str::from_utf8(target.triple()).map_err(|_| TyuError::NonUtf8Triple)?;
    let packs = discover_platforms_in(root)?;

    if let Some(pack) = packs.iter().find(|pack| {
        pack.manifest
            .platform
            .isa
            .iter()
            .any(|isa| isa.triple == triple)
    }) {
        if pack.manifest.platform.compiler_interface != RUNTIME_ABI_VERSION as u16 {
            return Err(TyuError::Platform(format!(
                "E{} pack={} isa={} detail=compiler-interface={} runtime-abi={}",
                E_PACK_INTERFACE_MISMATCH,
                pack.name(),
                triple,
                pack.manifest.platform.compiler_interface,
                RUNTIME_ABI_VERSION,
            )));
        }
    }

    Ok(())
}

pub fn format_lint_outcome(outcome: &LintOutcome) -> String {
    let mut out = String::new();
    if outcome.errors.is_empty() {
        let _ = writeln!(&mut out, "platform {}: ok", outcome.pack);
    } else {
        for err in &outcome.errors {
            let _ = writeln!(
                &mut out,
                "E{} pack={} detail={}",
                err.code, outcome.pack, err.detail
            );
        }
    }
    out
}

fn lint_pack_manifest(
    root: &Path,
    pack: &PlatformPack,
    all: bool,
) -> Result<LintOutcome, TyuError> {
    let mut errors = Vec::new();

    let manifest = &pack.manifest;
    let pack_name = pack.name().to_string();
    let pack_root = pack.manifest_path.parent().unwrap_or(root);
    if !all && errors.len() > 0 {
        return Ok(LintOutcome {
            pack: pack_name,
            errors,
        });
    }

    if manifest.platform.compiler_interface != RUNTIME_ABI_VERSION as u16 {
        errors.push(LintError::new(
            E_PACK_INTERFACE_MISMATCH,
            format!(
                "compiler-interface={} runtime-abi={}",
                manifest.platform.compiler_interface, RUNTIME_ABI_VERSION
            ),
        ));
        if !all {
            return Ok(LintOutcome {
                pack: pack_name,
                errors,
            });
        }
    }

    if let Err(e) = validate_file_size(&pack.manifest_path) {
        errors.push(e);
        if !all {
            return Ok(LintOutcome {
                pack: pack_name,
                errors,
            });
        }
    }

    let top_metal_root = validate_relative_path(pack_root, &manifest.metal.path)
        .map_err(|detail| TyuError::Platform(format!("{}: {}", pack.name(), detail)))?;

    for isa in &manifest.platform.isa {
        let metal = pack.effective_metal(isa);
        let metal_root = validate_relative_path(pack_root, &metal.path)
            .map_err(|detail| TyuError::Platform(format!("{}: {}", pack.name(), detail)))?;
        let startup_rel = validate_relative_path(&metal_root, &metal.startup)
            .map_err(|detail| TyuError::Platform(format!("{}: {}", pack.name(), detail)))?;
        if let Err(e) = validate_existing_file(&startup_rel) {
            errors.push(e);
            if !all {
                return Ok(LintOutcome {
                    pack: pack_name,
                    errors,
                });
            }
        }
        let linker_rel = if metal.linker.is_empty() {
            None
        } else {
            let rel = validate_relative_path(&metal_root, &metal.linker)
                .map_err(|detail| TyuError::Platform(format!("{}: {}", pack.name(), detail)))?;
            if let Err(e) = validate_existing_file(&rel) {
                errors.push(e);
                if !all {
                    return Ok(LintOutcome {
                        pack: pack_name,
                        errors,
                    });
                }
            }
            Some(rel)
        };

        let startup_text = fs::read_to_string(&startup_rel).map_err(|e| {
            TyuError::Platform(format!("reading '{}': {}", startup_rel.display(), e))
        })?;
        let exported = parse_exported_symbols(&startup_text);
        let required_symbols = required_symbols_for_pack(manifest, metal);
        for symbol in required_symbols {
            if !exported.contains(&symbol) {
                errors.push(LintError::new(
                    E_PACK_SYMBOL_MISSING,
                    format!("missing symbol '{}' in {}", symbol, startup_rel.display()),
                ));
                if !all {
                    return Ok(LintOutcome {
                        pack: pack_name,
                        errors,
                    });
                }
            }
        }

        if let Some(memory) = &manifest.memory {
            let linker_text = if let Some(linker_path) = &linker_rel {
                fs::read_to_string(linker_path).map_err(|e| {
                    TyuError::Platform(format!("reading '{}': {}", linker_path.display(), e))
                })?
            } else {
                String::new()
            };
            let regions = parse_linker_regions(&linker_text);
            for region in required_memory_regions(memory) {
                if !regions.contains(&region) {
                    errors.push(LintError::new(
                        E_PACK_SECTION_MISSING,
                        format!(
                            "missing memory region '{}' in {}",
                            region,
                            linker_rel
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "(no linker)".to_string())
                        ),
                    ));
                    if !all {
                        return Ok(LintOutcome {
                            pack: pack_name,
                            errors,
                        });
                    }
                }
            }
        }
    }

    for (feature, unit) in &manifest.features {
        let unit_path = validate_relative_path(&top_metal_root, &unit.unit)
            .map_err(|detail| TyuError::Platform(format!("{}: {}", pack.name(), detail)))?;
        if let Err(e) = validate_existing_file(&unit_path) {
            errors.push(LintError::new(E_PACK_FEATURE_UNIT_MISSING, e.detail));
            if !all {
                return Ok(LintOutcome {
                    pack: pack_name,
                    errors,
                });
            }
        }
        let _ = feature;
    }

    for (cap, glue) in &manifest.capabilities {
        if let Err(e) = lint_capability_glue(pack_root, cap, glue) {
            errors.push(e);
            if !all {
                return Ok(LintOutcome {
                    pack: pack_name,
                    errors,
                });
            }
        }
    }

    for isa in &manifest.platform.isa {
        let target = Target::parse(isa.triple.as_bytes())
            .ok_or_else(|| TyuError::Platform(format!("unknown target triple '{}'", isa.triple)))?;
        let want = compute_abi_hash(
            target.spec().calling_conv.arch_tag(),
            target.spec().slot_bytes,
            target.spec().word_bits,
            lmod::modinfo::MODINFO_VER,
        );
        match isa
            .expected_abi_hash
            .as_deref()
            .and_then(parse_expected_abi_hash_literal)
        {
            Some(got) if got == want => {}
            Some(got) => {
                errors.push(LintError::new(
                    E_PACK_ABI_HASH_MISMATCH,
                    format!(
                        "isa={} expected=0x{:016x} computed=0x{:016x}",
                        isa.triple, got, want
                    ),
                ));
                if !all {
                    return Ok(LintOutcome {
                        pack: pack_name,
                        errors,
                    });
                }
            }
            None => {
                errors.push(LintError::new(
                    E_PACK_ABI_HASH_MISMATCH,
                    format!("isa={} missing or invalid expected_abi_hash", isa.triple),
                ));
                if !all {
                    return Ok(LintOutcome {
                        pack: pack_name,
                        errors,
                    });
                }
            }
        }
    }

    match manifest.test.rung {
        TestRung::Untested => {}
        TestRung::Qemu | TestRung::Hardware => {
            if manifest
                .test
                .target
                .as_deref()
                .filter(|s| !s.is_empty())
                .is_none()
            {
                errors.push(LintError::new(
                    E_PACK_TESTRUNG_UNBACKED,
                    format!("rung={} missing target", manifest.test.rung.as_str()),
                ));
                if !all {
                    return Ok(LintOutcome {
                        pack: pack_name,
                        errors,
                    });
                }
            } else if let Some(target) = manifest.test.target.as_deref() {
                let target_path = validate_relative_path(root, target)
                    .map_err(|detail| TyuError::Platform(format!("{}: {}", pack.name(), detail)))?;
                if validate_existing_file(&target_path).is_err() {
                    errors.push(LintError::new(
                        E_PACK_TESTRUNG_UNBACKED,
                        format!("target '{}' not found", target_path.display()),
                    ));
                    if !all {
                        return Ok(LintOutcome {
                            pack: pack_name,
                            errors,
                        });
                    }
                }
            }
            if manifest
                .test
                .evidence
                .as_deref()
                .filter(|s| !s.is_empty())
                .is_none()
            {
                errors.push(LintError::new(
                    E_PACK_TESTRUNG_UNBACKED,
                    format!("rung={} missing evidence", manifest.test.rung.as_str()),
                ));
                if !all {
                    return Ok(LintOutcome {
                        pack: pack_name,
                        errors,
                    });
                }
            } else if let Some(evidence) = manifest.test.evidence.as_deref() {
                let evidence_path = pack_root.join(evidence);
                if !evidence_path.exists() {
                    errors.push(LintError::new(
                        E_PACK_TESTRUNG_UNBACKED,
                        format!("evidence '{}' not found", evidence_path.display()),
                    ));
                    if !all {
                        return Ok(LintOutcome {
                            pack: pack_name,
                            errors,
                        });
                    }
                }
            }
        }
    }

    match manifest.test.debug_agent.as_ref() {
        Some(debug_agent) if debug_agent.supported => {
            if debug_agent
                .target
                .as_deref()
                .filter(|s| !s.is_empty())
                .is_none()
            {
                errors.push(LintError::new(
                    E_PACK_DEBUG_AGENT_UNBACKED,
                    "debug-agent supported=true missing target",
                ));
                if !all {
                    return Ok(LintOutcome {
                        pack: pack_name,
                        errors,
                    });
                }
            }
            if debug_agent
                .evidence
                .as_deref()
                .filter(|s| !s.is_empty())
                .is_none()
            {
                errors.push(LintError::new(
                    E_PACK_DEBUG_AGENT_UNBACKED,
                    "debug-agent supported=true missing evidence",
                ));
                if !all {
                    return Ok(LintOutcome {
                        pack: pack_name,
                        errors,
                    });
                }
            } else if let Some(evidence) = debug_agent.evidence.as_deref() {
                let evidence_path = pack_root.join(evidence);
                if !evidence_path.exists() {
                    errors.push(LintError::new(
                        E_PACK_DEBUG_AGENT_UNBACKED,
                        format!(
                            "debug-agent evidence '{}' not found",
                            evidence_path.display()
                        ),
                    ));
                    if !all {
                        return Ok(LintOutcome {
                            pack: pack_name,
                            errors,
                        });
                    }
                }
            }
        }
        _ => {}
    }

    if let Some(deploy) = &manifest.deploy {
        if !matches!(
            deploy.method.as_str(),
            "qemu" | "elf-qemu" | "uf2" | "openocd"
        ) {
            errors.push(LintError::new(
                E_PACK_DEPLOY_RECIPE_INVALID,
                format!("unknown deploy method '{}'", deploy.method),
            ));
        }
    } else {
        errors.push(LintError::new(
            E_PACK_DEPLOY_RECIPE_INVALID,
            "missing [deploy] section",
        ));
    }

    Ok(LintOutcome {
        pack: pack_name,
        errors,
    })
}

fn required_symbols_for_pack(manifest: &PlatformManifest, metal: &MetalSection) -> Vec<String> {
    let mut symbols: Vec<String> = REQUIRED_BASE_SYMBOLS
        .iter()
        .map(|s| s.to_string())
        .collect();
    for s in &manifest.metal.required_symbols {
        if !symbols.contains(s) {
            symbols.push(s.clone());
        }
    }
    for s in &metal.required_symbols {
        if !symbols.contains(s) {
            symbols.push(s.clone());
        }
    }
    symbols
}

fn parse_exported_symbols(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let mut word = None;
        if let Some(rest) = trimmed.strip_prefix(".global ") {
            word = rest.split_whitespace().next();
        } else if let Some(rest) = trimmed.strip_prefix(".globl ") {
            word = rest.split_whitespace().next();
        } else if let Some(rest) = trimmed.strip_prefix("public ") {
            word = rest.split_whitespace().next();
        }
        if let Some(word) = word {
            if !word.is_empty() && !out.iter().any(|existing: &String| existing == word) {
                out.push(word.to_string());
            }
        }
    }
    out
}

fn parse_linker_regions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_memory = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("MEMORY") {
            in_memory = true;
            continue;
        }
        if in_memory && trimmed.starts_with('}') {
            break;
        }
        if !in_memory || trimmed.is_empty() || trimmed.starts_with("/*") {
            continue;
        }
        if let Some(name) = trimmed.split_whitespace().next() {
            if !name.is_empty() && !out.iter().any(|existing: &String| existing == name) {
                out.push(name.trim_end_matches(':').to_string());
            }
        }
    }
    out
}

fn required_memory_regions(memory: &MemorySection) -> Vec<String> {
    let mut regions = Vec::new();
    if let Some(flash) = &memory.flash {
        regions.push(flash.name.clone());
    }
    if let Some(sram) = &memory.sram {
        regions.push(sram.name.clone());
    }
    if let Some(ds_region) = &memory.ds_region {
        if !regions.contains(ds_region) {
            regions.push(ds_region.clone());
        }
    }
    regions
}

fn lint_capability_glue(root: &Path, cap: &str, glue: &CapabilityConfig) -> Result<(), LintError> {
    let required = capability_contract(cap).ok_or_else(|| {
        LintError::new(
            E_PACK_CAPABILITY_GLUE_MISSING,
            format!("unknown capability '{}'", cap),
        )
    })?;
    let glue_root = validate_relative_path(root, &glue.glue)
        .map_err(|detail| LintError::new(E_PACK_PATH_INVALID, detail.to_string()))?;
    let def_path = glue_root.with_extension("def");
    let mod_path = glue_root.with_extension("mod");
    if !def_path.is_file() {
        return Err(LintError::new(
            E_PACK_CAPABILITY_GLUE_MISSING,
            format!("missing '{}'", def_path.display()),
        ));
    }
    if !mod_path.is_file() {
        return Err(LintError::new(
            E_PACK_CAPABILITY_GLUE_MISSING,
            format!("missing '{}'", mod_path.display()),
        ));
    }
    let text = fs::read_to_string(&def_path).map_err(|e| {
        LintError::new(
            E_PACK_CAPABILITY_GLUE_MISSING,
            format!("reading '{}': {}", def_path.display(), e),
        )
    })?;
    let declared = parse_effect_words(&text);
    for (word, effect) in required.iter() {
        match declared.get(*word) {
            Some(found) if found == effect => {}
            Some(found) => {
                return Err(LintError::new(
                    E_PACK_CAPABILITY_GLUE_MISSING,
                    format!(
                        "{} effect mismatch: expected '{}' got '{}'",
                        word, effect, found
                    ),
                ));
            }
            None => {
                return Err(LintError::new(
                    E_PACK_CAPABILITY_GLUE_MISSING,
                    format!("missing '{}' in {}", word, def_path.display()),
                ));
            }
        }
    }
    Ok(())
}

fn parse_effect_words(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(':') {
            continue;
        }
        let rest = trimmed.trim_start_matches(':').trim();
        let Some(name_end) = rest.find(char::is_whitespace) else {
            continue;
        };
        let name = rest[..name_end].trim();
        let Some(open) = rest.find('(') else { continue };
        let Some(close) = rest[open + 1..].find(')') else {
            continue;
        };
        let effect = rest[open + 1..open + 1 + close].trim();
        if !name.is_empty() && !effect.is_empty() {
            out.insert(name.to_string(), effect.to_string());
        }
    }
    out
}

fn capability_contract(cap: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match cap {
        "gpio" => Some(&[
            ("platform.gpio.init", "usize usize --"),
            ("platform.gpio.write", "usize bool --"),
            ("platform.gpio.read", "usize -- bool"),
        ]),
        "uart" => Some(&[
            ("platform.uart.init", "usize --"),
            ("platform.uart.tx", "u8 --"),
            ("platform.uart.rx", "-- u8 bool"),
        ]),
        "time" => Some(&[
            ("platform.time.now_us", "-- i64"),
            ("platform.time.reboot", "--"),
        ]),
        _ => None,
    }
}

fn validate_relative_path(root: &Path, rel: &str) -> Result<PathBuf, TyuError> {
    let path = Path::new(rel);
    if path.is_absolute()
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::Prefix(_)
                    | std::path::Component::RootDir
            )
        })
    {
        return Err(TyuError::Platform(format!(
            "path '{}' escapes pack root",
            rel
        )));
    }
    Ok(root.join(path))
}

fn validate_existing_file(path: &Path) -> Result<(), LintError> {
    if !path.is_file() {
        return Err(LintError::new(
            E_PACK_MANIFEST_INVALID,
            format!("missing file '{}'", path.display()),
        ));
    }
    validate_file_size(path)
}

fn validate_file_size(path: &Path) -> Result<(), LintError> {
    let meta = fs::metadata(path).map_err(|e| {
        LintError::new(
            E_PACK_MANIFEST_INVALID,
            format!("metadata '{}': {}", path.display(), e),
        )
    })?;
    if meta.len() > MAX_PACK_FILE_BYTES {
        return Err(LintError::new(
            E_PACK_FILE_TOO_LARGE,
            format!(
                "file '{}' is {} bytes (> {})",
                path.display(),
                meta.len(),
                MAX_PACK_FILE_BYTES
            ),
        ));
    }
    Ok(())
}

fn parse_expected_abi_hash_literal(text: &str) -> Option<u64> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()
    } else {
        text.parse::<u64>().ok()
    }
}
