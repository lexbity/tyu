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
    /// P12.1 (§6.7): advisories that never fail the lint — today the
    /// "neither declared" model warning (§Q15: an unmodeled bundle is a
    /// first-class toolchain citizen, second-class evidence-chain citizen).
    pub warnings: Vec<String>,
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
// P12.1 (§6.7): the model-semantics lint matrix.
/// A non-`"unmodeled"` `model_semantics` id is declared but the `model/`
/// artifact is missing or declares a different id.
const E_PACK_MODEL_ARTIFACT_MISSING: u16 = 5413;
/// A `model/model.toml` artifact exists but the manifest declares no
/// `[model]` section (the artifact would silently never be used).
const E_PACK_MODEL_UNDECLARED_ARTIFACT: u16 = 5414;
/// A `[model]` value is outside its closed set (mmio / concurrency).
const E_PACK_MODEL_ENUM_INVALID: u16 = 5415;
/// A modeled bundle (`model_semantics` ≠ `"unmodeled"`) has no `evidence/`
/// vector corpus (required iff modeled, §6.7).
const E_PACK_MODEL_EVIDENCE_MISSING: u16 = 5416;

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
                warnings: Vec::new(),
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
    for warning in &outcome.warnings {
        let _ = writeln!(
            &mut out,
            "warning: pack={} detail={}",
            outcome.pack, warning
        );
    }
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
    if !all && !errors.is_empty() {
        return Ok(LintOutcome {
            pack: pack_name,
            errors,
            warnings: Vec::new(),
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
                warnings: Vec::new(),
            });
        }
    }

    if let Err(e) = validate_file_size(&pack.manifest_path) {
        errors.push(e);
        if !all {
            return Ok(LintOutcome {
                pack: pack_name,
                errors,
                warnings: Vec::new(),
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
                    warnings: Vec::new(),
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
                        warnings: Vec::new(),
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
                        warnings: Vec::new(),
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
                            warnings: Vec::new(),
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
                    warnings: Vec::new(),
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
                    warnings: Vec::new(),
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
                        warnings: Vec::new(),
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
                        warnings: Vec::new(),
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
                        warnings: Vec::new(),
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
                            warnings: Vec::new(),
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
                        warnings: Vec::new(),
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
                            warnings: Vec::new(),
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
                        warnings: Vec::new(),
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
                        warnings: Vec::new(),
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
                            warnings: Vec::new(),
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

    let mut warnings = Vec::new();
    lint_model_section(pack_root, manifest, &mut errors, &mut warnings);
    if !all && !errors.is_empty() {
        return Ok(LintOutcome {
            pack: pack_name,
            errors,
            warnings,
        });
    }

    Ok(LintOutcome {
        pack: pack_name,
        errors,
        warnings,
    })
}

/// The `[model]` pairing matrix (developer-proof-pipeline.md §6.7): the
/// violations + the §Q15 advisory, shared by the lint and the build-time
/// pairing gate (P12 finding 4c — `ensure_model_pairing`).
///
/// - declared non-`"unmodeled"` id without a matching `model/model.toml`
///   artifact ⇒ E5413; artifact declares a *different* id ⇒ E5413;
/// - declared `"unmodeled"` with a stray artifact declaring a real id ⇒
///   E5413 (the contradiction is a pairing violation, not a silent pass);
/// - `model/model.toml` artifact without any declared `[model]` ⇒ E5414;
/// - a `[model]` value outside its closed set ⇒ E5415;
/// - a `[model]` section under a schema < 3 ⇒ E5400 (the schema gate);
/// - `evidence/vectors.json` absent iff modeled ⇒ E5416 (the *corpus file*
///   specifically — `evidence/*.md` docs do not satisfy it, finding 4a);
/// - neither declared ⇒ warning only (§Q15: unmodeled is honest, not wrong).
fn model_pairing(pack_root: &Path, manifest: &PlatformManifest) -> (Vec<LintError>, Vec<String>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let model = match &manifest.model {
        None => {
            if model_artifact_declared_id(pack_root).is_some() {
                errors.push(LintError::new(
                    E_PACK_MODEL_UNDECLARED_ARTIFACT,
                    format!(
                        "model artifact '{}' declares an id but the manifest has no [model] section",
                        model_artifact_path(pack_root).display()
                    ),
                ));
            } else {
                warnings.push(
                    "model semantics not declared: bundle is unmodeled (§Q15) — obligations stay \
                     uncertifiable under --verify-policy=proven; declare [model] per \
                     developer-proof-pipeline.md §6.7"
                        .to_string(),
                );
            }
            return (errors, warnings);
        }
        Some(model) => model,
    };

    // Schema gate: declaring [model] requires the schema-3 stamp.
    if manifest.platform.schema != Some(crate::platform::MANIFEST_SCHEMA_MODEL) {
        errors.push(LintError::new(
            E_PACK_MANIFEST_INVALID,
            format!(
                "manifest declares [model] but schema is not {} (set schema = {})",
                crate::platform::MANIFEST_SCHEMA_MODEL,
                crate::platform::MANIFEST_SCHEMA_MODEL
            ),
        ));
    }

    // Closed value sets (§6.7). The descriptor parse gate enforces the same
    // sets for descriptor-bearing packs; the lint stays total either way.
    if !crate::platform::ModelSection::MMIO_VALUES.contains(&model.mmio_str()) {
        errors.push(LintError::new(
            E_PACK_MODEL_ENUM_INVALID,
            format!(
                "unknown [model] mmio '{}' (supported: {})",
                model.mmio_str(),
                crate::platform::ModelSection::MMIO_VALUES.join("|")
            ),
        ));
    }
    if !crate::platform::ModelSection::CONCURRENCY_VALUES.contains(&model.concurrency_str()) {
        errors.push(LintError::new(
            E_PACK_MODEL_ENUM_INVALID,
            format!(
                "unknown [model] concurrency '{}' (supported: {})",
                model.concurrency_str(),
                crate::platform::ModelSection::CONCURRENCY_VALUES.join("|")
            ),
        ));
    }

    if model.model_semantics == verifier::model::MODEL_UNMODELED {
        // Declared-unmodeled: honest tier — no model required, no evidence
        // required. BUT an artifact declaring a *real* id contradicts the
        // declaration (it would silently be dead — §6.7's
        // artifact-without-declaration pairing, applied to the unmodeled
        // declaration). An artifact declaring `id = "unmodeled"` is fine.
        if let Some(artifact_id) = model_artifact_declared_id(pack_root) {
            if artifact_id != verifier::model::MODEL_UNMODELED {
                errors.push(LintError::new(
                    E_PACK_MODEL_ARTIFACT_MISSING,
                    format!(
                        "[model] declares '{}' but the model artifact '{}' declares a real id '{}' \
                         (contradicts the unmodeled declaration; remove the artifact or declare \
                         model_semantics = \"{}\")",
                        verifier::model::MODEL_UNMODELED,
                        model_artifact_path(pack_root).display(),
                        artifact_id,
                        artifact_id
                    ),
                ));
            }
        }
        return (errors, warnings);
    }

    // Declared modeled: the model artifact must exist and declare the same id
    // (§6.7: "MUST match a model artifact directory (model/) with a model.toml
    // declaring the same id").
    let toml_path = model_artifact_path(pack_root);
    match model_artifact_declared_id(pack_root) {
        Some(artifact_id) => {
            if artifact_id != model.model_semantics {
                errors.push(LintError::new(
                    E_PACK_MODEL_ARTIFACT_MISSING,
                    format!(
                        "model artifact '{}' declares id '{}' but [model] declares '{}'",
                        toml_path.display(),
                        artifact_id,
                        model.model_semantics
                    ),
                ));
            }
        }
        None => {
            errors.push(LintError::new(
                E_PACK_MODEL_ARTIFACT_MISSING,
                format!(
                    "[model] declares '{}' but the model artifact '{}' is missing or declares no id",
                    model.model_semantics,
                    toml_path.display()
                ),
            ));
        }
    }

    // The bundle's evidence/ vector corpus is required iff modeled (§6.7):
    // a modeled bundle without conformance vectors would certify against an
    // unanchored model. The corpus is `evidence/vectors.json` SPECIFICALLY —
    // any other file in `evidence/` (docs, QEMU harness notes) does not
    // satisfy the requirement (P12 finding 4a: `evidence/*.md` alone must
    // lint as missing).
    if !pack_root.join("evidence").join("vectors.json").is_file() {
        errors.push(LintError::new(
            E_PACK_MODEL_EVIDENCE_MISSING,
            format!(
                "modeled bundle ({}) requires the evidence/vectors.json vector corpus (missing '{}')",
                model.model_semantics,
                pack_root.join("evidence").join("vectors.json").display()
            ),
        ));
    }
    (errors, warnings)
}

/// The lint form of the §6.7 model matrix.
fn lint_model_section(
    pack_root: &Path,
    manifest: &PlatformManifest,
    errors: &mut Vec<LintError>,
    warnings: &mut Vec<String>,
) {
    let (e, w) = model_pairing(pack_root, manifest);
    errors.extend(e);
    warnings.extend(w);
}

/// The **build-time** §6.7 pairing gate (P12 finding 4c): the evidence chain
/// keys on `pack.model_semantics()`; that id must be grounded — the declared
/// id paired to a matching artifact, the closed value sets honored, and the
/// `evidence/vectors.json` corpus present. In-tree packs are CI-guarded; this
/// is the structural protection for third-party packs: a pack cannot build as
/// "modeled" while its model is unanchored.
pub fn ensure_model_pairing(pack: &PlatformPack) -> Result<(), TyuError> {
    let (errors, _warnings) = model_pairing(pack.pack_root(), &pack.manifest);
    match errors.first() {
        None => Ok(()),
        Some(first) => Err(TyuError::Platform(format!(
            "pack={} model pairing (E{}): {} — run `tyu platform lint` for the full matrix",
            pack.name(),
            first.code,
            first.detail
        ))),
    }
}

/// The pack's `model/model.toml` path (the P12.2 model artifact: the `id`
/// the lint pairs with the declaration, plus the `[memory]` geometry that
/// the corpus generator and both bundle instances consume).
fn model_artifact_path(pack_root: &Path) -> PathBuf {
    pack_root.join("model").join("model.toml")
}

/// The `id` declared by `model/model.toml`, when the artifact exists and
/// parses.
fn model_artifact_declared_id(pack_root: &Path) -> Option<String> {
    parse_model_artifact(pack_root).map(|info| info.id)
}

/// The `model/model.toml` artifact shape (P12.1: identity; P12.2: the
/// `[memory]` geometry the bundle instance is derived from + the empty
/// refinement manifest). The lint enforces the declared↔artifact pairing;
/// the geometry is the single source the corpus generator, the Rust
/// `ApertureMem` instance, and the Lean bundle instance all consume.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelArtifactFile {
    model: ModelArtifactId,
    // Parsed-and-validated schema surface: a `[memory]` section must be
    // well-formed even though the window values are read from the bundled
    // parser (cleanup item 3).
    #[allow(dead_code)]
    #[serde(default)]
    memory: Option<ModelArtifactMemory>,
    // Parsed-and-validated schema surface today (the empty refinement
    // manifest must be accepted); P13 names refinements here.
    #[allow(dead_code)]
    #[serde(default)]
    refinements: Option<ModelArtifactRefinements>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelArtifactId {
    id: String,
}

/// The modeled RAM window (P12.2): the abstract model's point-store tracking
/// domain — `[origin, origin + length)` — consumed verbatim by the
/// `ApertureMem` Rust instance and the Lean `Tyu.Bundles` instance.
/// The modeled RAM window (P12.2): the `[memory] ram` table is parsed for
/// strict validation (a `[memory]` section with a malformed ram entry is a
/// malformed artifact); the *geometry values* are read from the single
/// `verifier::bundle::model_memory_ram` parser — the fields here carry the
/// deserialization so `deny_unknown_fields` stays total (cleanup item 3).
#[allow(dead_code)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelArtifactMemory {
    ram: ModelArtifactRam,
}

#[allow(dead_code)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelArtifactRam {
    // `name` is parsed-and-validated schema surface (the memory-region
    // name); the lint only consumes the window via the bundled parser.
    #[allow(dead_code)]
    name: String,
    origin: u64,
    length: u64,
}

/// The refinement manifest (P13 extends with named device refinements);
/// empty today — `[refinements]` present with no entries.
///
/// Parse-only schema surface: an empty `[refinements]` table must be
/// accepted (P12.2 ships the empty manifest; P13 adds named devices). The
/// struct carries no fields yet, so the dead-code allowance keeps the parse
/// gate honest without a phantom field.
#[allow(dead_code)]
#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ModelArtifactRefinements {}

/// The parsed model-artifact summary the lint exposes for the bundle layer.
/// The lint reads it to prove the artifact parses (identity and the
/// `[memory]`/`[refinements]` surface validate strictly) and to pin the
/// declared↔artifact pairing. The window geometry itself is read from the
/// SINGLE parser (`verifier::bundle::model_memory_ram` via
/// [`model_artifact_ram`]) so the evidence chain has exactly one geometry
/// source (cleanup item 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelArtifactInfo {
    pub id: String,
}

/// Parse the pack's `model/model.toml` artifact. `None` when the artifact is
/// absent or malformed (the lint reports E5413/E5414 on absent/mismatched;
/// the caller decides the error code).
pub fn parse_model_artifact(pack_root: &Path) -> Option<ModelArtifactInfo> {
    let text = std::fs::read_to_string(model_artifact_path(pack_root)).ok()?;
    let raw: ModelArtifactFile = toml::from_str(&text).ok()?;
    Some(ModelArtifactInfo { id: raw.model.id })
}

/// The declared RAM window of a modeled pack (`model/model.toml [memory]
/// ram`), when present — the geometry the bundle instances consume. Sourced
/// from the SINGLE parser (`verifier::bundle::model_memory_ram`), so the
/// tyu accessor, the corpus header, and the verifier suite cannot disagree
/// (cleanup item 3).
pub fn model_artifact_ram(pack_root: &Path) -> Option<(u64, u64)> {
    let text = std::fs::read_to_string(model_artifact_path(pack_root)).ok()?;
    verifier::bundle::model_memory_ram(&text)
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
