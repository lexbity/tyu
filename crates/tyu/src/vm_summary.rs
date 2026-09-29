//! The `tyu.vm/1` producer (PLAN-VERIFY-3 P11.1) — the verdicts-v2 →
//! manifest-summary converter.
//!
//! P11.1 says the `verify_manifest` record is written "from the build's
//! verdict set". This module is that producer: after a build, each module's
//! obligation artifact + verdicts (the harvest `tyu.verdicts/v2` docs when
//! `--verify-tool=lean` ran, else the in-tree verdicts echo) are converted
//! into the `tyu.vm/1` JSON summary — the exact document
//! `lmod_pack::verify::verify_manifest_from_json` consumes (the same values
//! the loader validates: ids, `id_hash`, status/trust per obligation,
//! `statement_hash`, counts, candidate ratio).
//!
//! Because the summary is *derived*, its `id_hash`/`statement_hash` values
//! are the real computed ones (recomputed through the canonical encoder when
//! a verdict record does not carry them), never hand-authored constants. The
//! emitter writes one `<Module>.vm.json` beside the report
//! (`<out>/.tyu-verify/`), and the manifest/assembly paths (lmod-pack,
//! deploy) consume those instead of requiring a hand-written file.

use std::fs;
use std::path::{Path, PathBuf};

use verifier::model::OblSet;
use verifier::stmt::{sha256_hex16, StatementContext};
use verifier::verdict::{Trust, VerdictStatus, Verdicts};

use crate::args::VerifyPolicy;
use crate::error::TyuError;
use crate::proof::VerifyEnvKey;

const VM_SCHEMA: &str = "tyu.vm/1";

/// The verification-state directory under the out dir: `<out>/.tyu-verify/`.
pub fn verify_state_dir(out_dir: &Path) -> PathBuf {
    out_dir.join(".tyu-verify")
}

/// The per-module summary path: `<out>/.tyu-verify/<Module>.vm.json`.
pub fn vm_summary_path(out_dir: &Path, module: &str) -> PathBuf {
    verify_state_dir(out_dir).join(format!("{module}.vm.json"))
}

/// The harvest verdicts doc path for a module (`<out>/.tyu-verify/harvest/
/// <Module>.verdicts.v2.json`), when the proof pipeline produced one.
pub fn harvest_verdicts_path(out_dir: &Path, module: &str) -> PathBuf {
    verify_state_dir(out_dir)
        .join("harvest")
        .join(format!("{module}.verdicts.v2.json"))
}

/// The verdicts source for a module: the harvest doc (post-certificate
/// `--verify-tool=lean`) when present, else the in-tree echo slot. Both are
/// `tyu.verdicts/v2`.
///
/// **Identity gate (P12 finding 3 / §Q3):** a module-name-keyed harvest doc
/// from an earlier build is only reusable under the CURRENT artifact's
/// identity. If the doc's recorded `(target, model_semantics)` differs from
/// the artifact's, it is *stale* — fail-closed to `None` (every obligation
/// reads open in the summary; the E6421-class diagnostic is emitted), never
/// silently reused. The elision path enforces the same rule independently
/// (langc E6421); this is the package/summary leg.
pub fn verdicts_for_module(
    out_dir: &Path,
    module: &str,
    obl_path: &Path,
    verify_env: &VerifyEnvKey,
) -> Result<Option<Verdicts>, TyuError> {
    // The artifact's identity is the ground truth the verdicts must match.
    let (art_target, art_model) = artifact_identity(obl_path)?;
    let stale = |v: &Verdicts| v.target != art_target || v.model_semantics != art_model;
    let reject_stale = |v: Verdicts, src: &Path| -> Option<Verdicts> {
        if stale(&v) {
            eprintln!(
                "tyu: error[E6421]: harvest verdicts '{}' stale: recorded ({}, {}) != artifact ({}, {}) — \
                 verdicts ignored, obligations open (checks retained)",
                src.display(),
                v.target,
                v.model_semantics,
                art_target,
                art_model
            );
            return None;
        }
        Some(v)
    };

    // 1. The harvest doc (the certificate post-pipeline state).
    let harvest = harvest_verdicts_path(out_dir, module);
    if harvest.is_file() {
        let bytes = fs::read(&harvest).map_err(TyuError::Io)?;
        let v = verifier::verdict::read_verdicts(&bytes).map_err(|e| {
            TyuError::Build(format!(
                "E6416: harvest verdicts '{}' unreadable for the manifest summary: {e:?}",
                harvest.display()
            ))
        })?;
        return Ok(reject_stale(v, &harvest));
    }
    // 2. The in-tree echo slot (the langc verdicts store).
    if let Some(echo_path) = echo_path_for(obl_path, verify_env) {
        if let Ok(bytes) = fs::read(&echo_path) {
            if let Ok(echo) = verifier::verdict::read_echo(&bytes) {
                return Ok(reject_stale(echo.verdicts, &echo_path));
            }
        }
    }
    Ok(None)
}

/// The obligation artifact's `(target, model_semantics)` identity — the
/// consuming build's pair (§Q3).
fn artifact_identity(obl_path: &Path) -> Result<(String, String), TyuError> {
    let bytes = fs::read(obl_path).map_err(TyuError::Io)?;
    let set = verifier::codec::read_obl(&bytes).map_err(|e| {
        TyuError::Build(format!(
            "vm summary: artifact '{}' invalid (E{}): {e:?}",
            obl_path.display(),
            e.code()
        ))
    })?;
    Ok((set.target, set.model_semantics))
}

/// The langc verdicts-echo slot for an obligation artifact (mirrors
/// `verify.rs::echo_path_for`; exposed here so the summary source is the
/// same document the report sources).
fn echo_path_for(obl_path: &Path, verify_env: &VerifyEnvKey) -> Option<PathBuf> {
    let file = obl_path.file_name()?.to_str()?;
    let stem = file.strip_suffix(".obl.json")?;
    let (module, fp) = stem.rsplit_once('-')?;
    if fp.len() != 16 || !fp.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let slot =
        crate::proof::verdicts_slot_name(module, u64::from_str_radix(fp, 16).ok()?, verify_env);
    Some(obl_path.parent()?.join(".tyu-verify").join(slot))
}

/// JSON-value helpers (hand-rolled; FR-15).
fn jstr(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The `tyu.vm/1` document for one module, derived from its obligation
/// artifact + verdicts. `policy` is the build's policy (the manifest's
/// declared policy). Values are the real computed ones.
pub fn vm_document(
    module: &str,
    target: &str,
    model: &str,
    policy: VerifyPolicy,
    set: &OblSet,
    verdicts: &Verdicts,
) -> Result<String, TyuError> {
    // Per-obligation entries, canonicalized (ids sorted — the loader
    // requires non-decreasing ids).
    let mut rows: Vec<(String, String, VerdictStatus, Trust, String)> = Vec::new();
    for o in &set.obligations {
        let word_ir_hash = set
            .facts
            .words
            .iter()
            .find(|w| w.name == o.site.word)
            .map(|w| sha256_hex16(w.ir.as_bytes()))
            .unwrap_or_default();
        let ctx = StatementContext::for_obligation(module, target, model, &word_ir_hash, o);
        let computed = ctx.statement_hash_hex(&o.formula);
        match verdicts.lookup(&o.id, &o.id_hash) {
            Some(r) => {
                let stmt = r.statement_hash.clone().unwrap_or(computed);
                rows.push((o.id.clone(), o.id_hash.clone(), r.status, r.trust, stmt));
            }
            // No record for the obligation: it was not discharged — open
            // (fail-closed: the manifest says the check is retained).
            None => rows.push((
                o.id.clone(),
                o.id_hash.clone(),
                VerdictStatus::Open,
                Trust::Open,
                computed,
            )),
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    // Counts by trust class: [proof, checked, assumed, open].
    let mut counts = [0u32; 4];
    let mut proofs = 0u32;
    let candidates = 0u32;
    for r in &rows {
        match r.3 {
            Trust::Proof => {
                counts[0] += 1;
                proofs += 1;
            }
            Trust::Checked => counts[1] += 1,
            Trust::Assumed => counts[2] += 1,
            Trust::Open => counts[3] += 1,
        }
    }
    // candidate_ratio: basis 10000 (§Q10) — candidate-authored proofs over
    // all proofs.
    let candidate_ratio: u16 = if proofs == 0 {
        0
    } else {
        ((candidates as u64).saturating_mul(10_000) / proofs as u64) as u16
    };

    // certifier identity: the verdicts file's producer — class "port" only
    // for a RECOGNIZED certifier (the harvest's `tyu-port/lean/1`); the
    // in-tree echo names no recognized producer → class "none".
    let (cert_class, cert_name, cert_rec): (&str, &str, &str) = match &verdicts.certifier {
        Some(c) if verifier::verdict::RECOGNIZED_CERTIFIERS.contains(&c.recognition.as_str()) => {
            ("port", &c.name, &c.recognition)
        }
        _ => ("none", "", ""),
    };

    let policy_s = policy.as_str();
    let status_s = |s: VerdictStatus| s.as_str();
    let trust_s = |t: Trust| t.as_str();

    let mut out = String::with_capacity(512 + rows.len() * 160);
    out.push_str("{\"schema\":");
    jstr(&mut out, VM_SCHEMA);
    out.push_str(",\"semantics\":");
    jstr(&mut out, verifier::semantics::SEMANTICS_VERSION);
    out.push_str(",\"stmt\":");
    jstr(&mut out, verifier::stmt::STMT_SCHEMA);
    out.push_str(",\"target\":");
    jstr(&mut out, target);
    out.push_str(",\"model\":");
    jstr(&mut out, model);
    out.push_str(",\"policy\":");
    jstr(&mut out, policy_s);
    out.push_str(",\"certifier\":{\"class\":");
    jstr(&mut out, cert_class);
    out.push_str(",\"name\":");
    jstr(&mut out, cert_name);
    out.push_str(",\"recognition\":");
    jstr(&mut out, cert_rec);
    out.push_str("},\"candidate_ratio\":");
    out.push_str(&candidate_ratio.to_string());
    out.push_str(",\"counts\":{\"proof\":");
    out.push_str(&counts[0].to_string());
    out.push_str(",\"checked\":");
    out.push_str(&counts[1].to_string());
    out.push_str(",\"assumed\":");
    out.push_str(&counts[2].to_string());
    out.push_str(",\"open\":");
    out.push_str(&counts[3].to_string());
    out.push_str("},\"obligations\":[");
    for (i, (id, id_hash, status, trust, stmt)) in rows.iter().enumerate() {
        if i != 0 {
            out.push(',');
        }
        out.push_str("{\"id\":");
        jstr(&mut out, id);
        out.push_str(",\"id_hash\":");
        // id_hash is the fnv1a64 identity key as a hex16 string; the
        // manifest carries the numeric value.
        let idh = u64::from_str_radix(id_hash, 16).unwrap_or(0);
        out.push_str(&idh.to_string());
        out.push_str(",\"status\":");
        jstr(&mut out, status_s(*status));
        out.push_str(",\"trust\":");
        jstr(&mut out, trust_s(*trust));
        out.push_str(",\"statement_hash\":");
        jstr(&mut out, stmt);
        out.push('}');
    }
    out.push_str("]}\n");
    Ok(out)
}

/// Write the per-module `tyu.vm/1` summaries for a build (one beside the
/// report, deterministic). Returns `(module, path)` for every module that
/// produced a summary. `verdicts` resolution per module: harvest doc → echo
/// slot.
pub fn write_module_summaries(
    out_dir: &Path,
    module_obl: &[(String, Option<PathBuf>)],
    policy: VerifyPolicy,
    verify_env: &VerifyEnvKey,
) -> Result<Vec<(String, PathBuf)>, TyuError> {
    let mut written = Vec::new();
    let state_dir = verify_state_dir(out_dir);
    fs::create_dir_all(&state_dir).map_err(TyuError::Io)?;
    for (module, maybe_obl) in module_obl {
        let Some(obl_path) = maybe_obl else {
            continue;
        };
        let bytes = fs::read(obl_path).map_err(TyuError::Io)?;
        let set = verifier::codec::read_obl(&bytes).map_err(|e| {
            TyuError::Build(format!(
                "vm summary: module {module} artifact invalid (E{}): {e:?}",
                e.code()
            ))
        })?;
        let target = set.target.clone();
        let model = set.model_semantics.clone();
        let verdicts = match verdicts_for_module(out_dir, module, obl_path, verify_env)? {
            Some(v) => v,
            None => Verdicts {
                semantics: verifier::semantics::SEMANTICS_VERSION.to_string(),
                stmt: verifier::stmt::STMT_SCHEMA.to_string(),
                certifier: None,
                target: target.clone(),
                model_semantics: model.clone(),
                concurrency: set.concurrency.clone(),
                records: Vec::new(),
            },
        };
        let doc = vm_document(module, &target, &model, policy, &set, &verdicts)?;
        let path = vm_summary_path(out_dir, module);
        write_if_changed(&path, doc.as_bytes()).map_err(TyuError::Io)?;
        written.push((module.clone(), path));
    }
    written.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(written)
}

/// Atomic write-if-changed (same discipline as the proof-package writer):
/// unchanged content is not rewritten (determinism + mtime stability).
fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Ok(existing) = fs::read(path) {
        if existing == bytes {
            return Ok(());
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use verifier::codec::encode_obl;
    use verifier::model::OblSet;

    const ART_TARGET: &str = "x86_64-unknown-none";
    const ART_MODEL: &str = "tyu.model/x86_64-unknown-none/1";

    fn artifact(dir: &Path) -> PathBuf {
        let obl = dir.join("Main.obl.json");
        let set: OblSet = OblSet {
            schema: "tyu.obl/v2".to_string(),
            semantics: verifier::semantics::SEMANTICS_VERSION.to_string(),
            stmt: verifier::stmt::STMT_SCHEMA.to_string(),
            module: "Main".to_string(),
            target: ART_TARGET.to_string(),
            platform: ART_TARGET.to_string(),
            model_semantics: ART_MODEL.to_string(),
            concurrency: verifier::model::CONCURRENCY_UNMODELED.to_string(),
            abi_contract_version: ir::contract::ABI_CONTRACT_VERSION as u32,
            facts: verifier::model::Facts {
                words: Vec::new(),
                subtypes: Vec::new(),
                predicates: Vec::new(),
            },
            obligations: Vec::new(),
        };
        fs::write(&obl, encode_obl(&set).unwrap()).unwrap();
        obl
    }

    fn harvest(dir: &Path, target: &str, model: &str) {
        let doc = verifier::verdict::encode_verdicts(
            "harvest",
            "0.1.0",
            None,
            target,
            model,
            "unmodeled",
            &[],
            0,
            &verifier::verdict::EmittedChecksData {
                subtype_range: 0,
                contract: 0,
                mmio_bounds: 0,
            },
        )
        .unwrap();
        fs::write(harvest_verdicts_path(dir, "Main"), &doc).unwrap();
    }

    /// The identity gate over one `(harvest target, harvest model)` case:
    /// only a doc whose recorded pair equals the artifact's pair is reused.
    fn case(harvest_target: &str, harvest_model: &str) -> bool {
        let dir = std::env::temp_dir().join(format!(
            "tyu_vm_summary_{}_{}",
            std::process::id(),
            harvest_target
                .len()
                .wrapping_mul(31)
                .wrapping_add(harvest_model.len())
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".tyu-verify").join("harvest")).unwrap();
        let obl = artifact(&dir);
        harvest(&dir, harvest_target, harvest_model);
        let env = VerifyEnvKey::compute(
            &dir,
            ART_MODEL,
            crate::proof::refinements_hash_for_model(ART_MODEL),
            crate::proof::concurrency_hash_for_model(ART_MODEL),
        );
        verdicts_for_module(&dir, "Main", &obl, &env)
            .unwrap()
            .is_some()
    }

    /// P12 finding 5 / §Q3 — a doc recorded under a *different model* (same
    /// target) is stale: fail-closed to `None` (obligations open).
    #[test]
    fn stale_harvest_under_another_model_is_rejected() {
        assert!(
            !case(ART_TARGET, "tyu.model/some-other-bundle/9"),
            "model-only mismatch must be rejected (stale → open)"
        );
    }

    /// P12 finding 5 / §Q3 — a doc recorded under a *different target* (same
    /// model) is equally stale.
    #[test]
    fn stale_harvest_under_another_target_is_rejected() {
        assert!(
            !case("riscv32-unknown-none", ART_MODEL),
            "target-only mismatch must be rejected (stale → open)"
        );
    }

    /// The matching-identity doc is reused (the normal path — no false
    /// rejection).
    #[test]
    fn matching_identity_harvest_is_reused() {
        assert!(
            case(ART_TARGET, ART_MODEL),
            "the matching-identity harvest doc is reused"
        );
    }
}
