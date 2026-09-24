//! Image-level verification composition (static-verification.md §7.3, slice
//! P3/P4, as amended).
//!
//! After every module compiles, `tyu` composes `verify-report.json` from each
//! module's `.obl.json` artifact plus its **verdicts echo**
//! (`<Module>.verdicts.inTree.json`, re-homed into `.tyu-verify/` §7.4):
//!
//! - per-module class accounting over the five obligation kinds, resolved from
//!   the echo records (discharged/assumed) — obligations the echo does not
//!   mention are open (P4; the P3 descriptor rule is the *fallback* for
//!   modules without an echo);
//! - `stack-budget(main)` and `stack-budget(isr)` verdicts (§7.3):
//!   `N_main` is **derived** from the runtime binary's own data-stack
//!   geometry (`__lang_ds_limit − __lang_ds_base` over `slot_bytes` — bounds
//!   are computed, never hand-declared, stack-bound-analysis §13); `N_isr`
//!   is the one **declared** descriptor grant (default 32, FR-10);
//! - `mmio-bounds` obligations discharged by the deterministic descriptor
//!   rule (Q8) — the exact arithmetic the runtime bounds check performs;
//! - the `emitted_checks` honesty block (FR-15/FR-16): how many runtime checks
//!   of each class actually made it into the object, summed from the per-module
//!   echoes langc wrote under codegen (`--verify=off` keeps every check, so
//!   the echo's emitted counts equal the totals);
//! - `open`/`assumed` lists, `stale_verdicts`, trusted assumptions (FR-15);
//! - E6410 policy enforcement (`--verify-policy=no-open[,-no-assumptions]`,
//!   FR-18) and the NFR-9 one-line accounting summary.
//!
//! Budgets are per-context (Q5 — the design rejects the cross-context sum);
//! the ISR data stack is a separate region, so no `main + isr` addition
//! appears anywhere.

use std::path::{Path, PathBuf};

use verifier::codec::read_obl;
use verifier::model::{Formula, Kind, Obligation, OblSet};
use verifier::report::{
    AssumedObligation, ClassAccounting, EmittedChecks, IsrContextAccounting,
    MainContextAccounting, ModuleAccounting, OpenObligation, StackContextAccounting,
    ToolInfo, TrustedAssumption, VerifyReport,
};
use verifier::verdict::{read_echo, Echo, VerdictStatus};

use crate::args::{VerifyMode, VerifyPolicy};
use crate::build::BuildContext;
use crate::error::TyuError;

const KIND_ORDER: [&str; 5] = [
    Kind::SubtypeRange.as_str(),
    Kind::ContractPre.as_str(),
    Kind::ContractPost.as_str(),
    Kind::StackBudget.as_str(),
    Kind::MmioBounds.as_str(),
];

/// Compose and write `<out_dir>/verify-report.json`, enforce the build policy
/// (E6410, FR-18), and print the NFR-9 one-line accounting summary.
pub fn compose_and_write_report(
    ctx: &BuildContext,
    module_obl: &[(String, Option<PathBuf>)],
    root_module: Option<&str>,
    verify: VerifyMode,
    policy: VerifyPolicy,
) -> Result<PathBuf, TyuError> {
    let report = compose(ctx, module_obl, root_module, verify, policy)?;
    let bytes = verifier::codec::encode_report(&report)
        .map_err(|e| TyuError::Build(format!("verify-report encode failed: {e:?}")))?;
    let path = ctx.out_dir.join("verify-report.json");
    std::fs::write(&path, &bytes)
        .map_err(|e| TyuError::Build(format!("writing '{}': {}", path.display(), e)))?;
    // E6410 policy gate (FR-18): a no-open policy fails the build listing the
    // open (and, with -no-assumptions, assumed) obligations. Runs AFTER the
    // report is written so a failed build still leaves the diagnostics.
    // Under `--verify=off` the policy is moot (every check is emitted and the
    // report documents `policy: "off"`); the gate does not apply.
    if verify == VerifyMode::On {
        enforce_policy(&report, policy)?;
    }
    // NFR-9: one-line accounting summary on stderr.
    let (total, discharged, assumed, open) = report_totals(&report);
    eprintln!(
        "verify: {} obligations — {} discharged, {} assumed, {} open (report: {})",
        total,
        discharged,
        assumed,
        open,
        path.display()
    );
    Ok(path)
}

/// Compose the report model (§7.3 + §6.5 P4 fields).
pub fn compose(
    ctx: &BuildContext,
    module_obl: &[(String, Option<PathBuf>)],
    root_module: Option<&str>,
    verify: VerifyMode,
    policy: VerifyPolicy,
) -> Result<VerifyReport, TyuError> {
    // Load every module artifact + its verdicts echo; a missing one
    // (mixed-mode / pre-P4 cache) degrades to the P3 fallback, never a crash
    // (§7.3).
    let mut sets: Vec<(String, Option<OblSet>)> = Vec::new();
    let mut echoes: Vec<(String, Option<Echo>)> = Vec::new();
    for (name, maybe_path) in module_obl {
        let set = match maybe_path {
            Some(path) => load_obl(path)?,
            None => None,
        };
        let echo = maybe_path
            .as_deref()
            .and_then(echo_path_for)
            .map(|p| load_echo(&p))
            .transpose()?
            .flatten();
        sets.push((name.clone(), set));
        echoes.push((name.clone(), echo));
    }

    // Per-context budgets (amended §6.4): N_main derived from the runtime
    // binary's geometry; N_isr declared in the platform pack (default 32).
    let n_main = derive_main_budget(ctx);
    let n_isr = verification_isr_grant(ctx)?;

    // §7.3 accounting.
    let main = main_context(&sets, root_module, n_main);
    let isr = isr_context(&sets, n_isr);

    let policy_str = match verify {
        VerifyMode::On => policy.as_str().to_string(),
        // --verify=off keeps every check; the report documents the mode.
        VerifyMode::Off => "off".to_string(),
    };
    let mut report = VerifyReport::new(
        ToolInfo {
            name: "tyu".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        &policy_str,
    );

    // Per-module class accounting + open/assumed lists + emitted/stale sums.
    let mut stale_verdicts: u32 = 0;
    let mut emitted = EmittedChecks::default();
    emitted.data_stack_guards = true; // P7 elides the x86 data-stack guard.
    for (i, (name, set)) in sets.iter().enumerate() {
        let echo = echoes[i].1.as_ref();
        let classes = classes_for(set.as_ref(), echo);
        report.modules.push(ModuleAccounting {
            name: name.clone(),
            classes,
        });
        if let Some(set) = set {
            collect_open(set, echo, name, &mut report.open);
            collect_assumed(set, echo, name, &mut report.assumed);
        }
        // The honest emitted-check block: langc counted what it actually
        // emitted per word. Fallback (no echo: legagcy artifact): every
        // subtype/mmio site emitted, contract unaccounted (0) — the echo
        // exists for every P4 build, so this only affects pre-P4 caches.
        if let Some(e) = echo {
            emitted.subtype_range += e.emitted.subtype_range;
            emitted.contract += e.emitted.contract;
            emitted.mmio_bounds += e.emitted.mmio_bounds;
            stale_verdicts += e.stale_verdicts;
        } else if let Some(set) = set {
            let sub = set
                .obligations
                .iter()
                .filter(|o| o.kind == Kind::SubtypeRange)
                .count() as u32;
            let mmio = set
                .obligations
                .iter()
                .filter(|o| o.kind == Kind::MmioBounds)
                .count() as u32;
            emitted.subtype_range += sub;
            emitted.mmio_bounds += mmio;
        }
    }
    report.open.sort_by(|a, b| a.module.cmp(&b.module).then_with(|| a.id.cmp(&b.id)));
    report.assumed.sort_by(|a, b| a.module.cmp(&b.module).then_with(|| a.id.cmp(&b.id)));

    report.contexts = StackContextAccounting {
        main: main.clone(),
        isr: isr.clone(),
        guards: "retained".to_string(),
    };
    report.stale_verdicts = stale_verdicts;
    report.emitted_checks = emitted;

    // Trusted facts used by discharges (FR-15 / T2) — copied at composition
    // time, in deterministic order. The ISR budget is a *declared descriptor*
    // grant (trusted); the derived main geometry is listed as a *runtime*
    // fact for transparency (it is abi-hash-checked, not trusted-on-faith).
    if main.verdict == "discharged" && main.budget > 0 {
        report
            .assumptions_trusted
            .push(TrustedAssumption {
                kind: "runtime".to_string(),
                what: format!(
                    "derived N_main={} ({} bytes / {} slot_bytes from __lang_ds_base/__lang_ds_limit)",
                    main.budget,
                    main.budget as u64 * ctx.target.spec().slot_bytes as u64,
                    ctx.target.spec().slot_bytes,
                ),
            });
    }
    if isr.verdict == "discharged" && isr.handlers > 0 {
        report
            .assumptions_trusted
            .push(TrustedAssumption {
                kind: "descriptor".to_string(),
                what: format!("isr_stack_slots={}", isr.budget),
            });
    }
    // The aperture sizes used by mmio-bounds discharges come from each
    // record's own `assumptions` (T2) — the composition re-reads them.
    for (name, set) in &sets {
        if let Some(set) = set {
            collect_mmio_sizes(set, name, &mut report.assumptions_trusted);
        }
    }
    report.assumptions_trusted.sort_by(|a, b| a.what.cmp(&b.what));

    Ok(report)
}

/// The module's verdicts-echo path, derived from its re-homed obl path:
/// `<out_dir>/<Module>-<fp>.obl.json` → `<out_dir>/.tyu-verify/<Module>-<fp>.verdicts.json`.
fn echo_path_for(obl_path: &Path) -> Option<PathBuf> {
    let file = obl_path.file_name()?.to_str()?;
    let stem = file.strip_suffix(".obl.json")?;
    Some(
        obl_path
            .parent()?
            .join(".tyu-verify")
            .join(format!("{stem}.verdicts.json")),
    )
}

/// Read (and fail-closed validate) one module's `.obl.json`; a missing file
/// is facts-unavailable, a malformed one is a hard build error (E6401-class:
/// the artifact was produced moments ago by langc, so corruption is a real
/// fault, not a mode).
fn load_obl(path: &Path) -> Result<Option<OblSet>, TyuError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path)
        .map_err(|e| TyuError::Build(format!("reading '{}': {}", path.display(), e)))?;
    read_obl(&bytes).map(Some).map_err(|e| {
        TyuError::Build(format!(
            "obligation artifact '{}' is invalid (E{}): {e:?}",
            path.display(),
            e.code()
        ))
    })
}

/// Read (and fail-closed validate) one module's verdicts echo. A missing file
/// is the no-echo fallback (legacy/mixed-mode); a malformed one is a hard
/// build error (E6402-class — produced moments ago by langc).
fn load_echo(path: &Path) -> Result<Option<Echo>, TyuError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path)
        .map_err(|e| TyuError::Build(format!("reading '{}': {}", path.display(), e)))?;
    read_echo(&bytes).map(Some).map_err(|e| {
        TyuError::Build(format!(
            "verdicts echo '{}' is invalid (E{}): {e:?}",
            path.display(),
            e.code()
        ))
    })
}

/// The declared ISR grant from the build's compiled descriptor (amended
/// §6.4). No platform selection → the default 32 (FR-10) — the ISR grant is
/// a policy number, meaningful without a board descriptor.
fn verification_isr_grant(ctx: &BuildContext) -> Result<u32, TyuError> {
    let Some(selection) = ctx.platform_selection() else {
        return Ok(codegen_core::compiled_desc::DEFAULT_ISR_STACK_SLOTS);
    };
    let compiled = crate::platform::desc::compile::ensure_compiled_descriptor(
        &selection.pack.manifest_path,
        selection.pack.pack_root(),
    )?;
    Ok(compiled.verification.isr_stack_slots)
}

/// Derive the main context's `N_main` from the runtime binary's own
/// data-stack geometry (amended §6.4): the runtime object that was just
/// assembled into `out_dir` defines `__lang_ds_base` / `__lang_ds_limit` as
/// BSS labels, so the difference of their section-relative symbol values is
/// the reservation in bytes regardless of the final link address. Divided by
/// the target's `slot_bytes`, that is the main context's slot budget.
///
/// `None` (→ `stack-budget(main)` open, fail-closed) when the runtime
/// artifact is absent or does not define the geometry symbols: absence can
/// only ever cause *more* checking, never less (§7.5 discipline).
fn derive_main_budget(ctx: &BuildContext) -> Option<u32> {
    let obj_path = ctx.out_dir.join("runtime.o");
    let data = std::fs::read(&obj_path).ok()?;
    let base = crate::elf_reader::symbol_value(&data, b"__lang_ds_base")?;
    let limit = crate::elf_reader::symbol_value(&data, b"__lang_ds_limit")?;
    if limit <= base {
        return None;
    }
    let slot_bytes = ctx.target.spec().slot_bytes as u64;
    if slot_bytes == 0 {
        return None;
    }
    let slots = (limit - base) / slot_bytes;
    if slots == 0 || slots > u32::MAX as u64 {
        return None;
    }
    Some(slots as u32)
}

/// The main context verdict (§7.3): `high(main) ≠ ⊤ ∧ high(main) ≤ N_main`.
fn main_context(
    sets: &[(String, Option<OblSet>)],
    root_module: Option<&str>,
    n_main: Option<u32>,
) -> MainContextAccounting {
    let Some(root) = root_module else {
        return MainContextAccounting {
            high: 0,
            top: false,
            budget: 0,
            verdict: "n/a".to_string(),
        };
    };
    let root_set = sets.iter().find(|(n, _)| n == root).and_then(|(_, s)| s.as_ref());
    let main_word = root_set
        .and_then(|set| set.facts.words.iter().find(|w| w.name == "main"));
    let Some(word) = main_word else {
        return MainContextAccounting {
            high: 0,
            top: false,
            budget: n_main.unwrap_or(0),
            verdict: "open".to_string(),
        };
    };
    if word.top {
        return MainContextAccounting {
            high: word.high,
            top: true,
            budget: n_main.unwrap_or(0),
            verdict: "open".to_string(),
        };
    }
    let Some(budget) = n_main else {
        // Runtime geometry unavailable (no runtime artifact or missing DS
        // symbols) — open-with-reason, fail-closed (§7.3: absence can only
        // ever cause more checking, never less).
        return MainContextAccounting {
            high: word.high,
            top: false,
            budget: 0,
            verdict: "open".to_string(),
        };
    };
    let verdict = if word.high <= budget {
        "discharged".to_string()
    } else {
        "open".to_string()
    };
    MainContextAccounting {
        high: word.high,
        top: false,
        budget,
        verdict,
    }
}

/// The ISR context verdict (§7.3): every handler word must satisfy
/// `high ≠ ⊤ ∧ high ≤ N_isr`. On a successful build these are already proven
/// by E5030 against the same descriptor grant, so the verdict records the
/// exact check as discharged; the report surfaces the max peak + count.
fn isr_context(sets: &[(String, Option<OblSet>)], n_isr: u32) -> IsrContextAccounting {
    let handlers: Vec<&verifier::model::WordFact> = sets
        .iter()
        .filter_map(|(_, s)| s.as_ref())
        .flat_map(|set| set.facts.words.iter())
        .filter(|w| w.performs.iter().any(|p| p == "INTERRUPT"))
        .collect();
    if handlers.is_empty() {
        return IsrContextAccounting {
            max_high: 0,
            budget: n_isr,
            handlers: 0,
            verdict: "n/a".to_string(),
        };
    }
    let max_high = handlers.iter().map(|w| w.high).max().unwrap_or(0);
    IsrContextAccounting {
        max_high,
        budget: n_isr,
        handlers: handlers.len() as u32,
        verdict: "discharged".to_string(),
    }
}

/// The build-time verdict of one obligation (P4): the module's echo is
/// authoritative — a record matching `(id, id_hash)` decides, absence is
/// open (fail-closed, Q3). Without an echo (legacy/mixed-mode artifacts) the
/// P3 descriptor rule discharges the provably-fitting mmio-bounds sites and
/// everything else is open.
fn resolved_status(o: &Obligation, echo: Option<&Echo>) -> VerdictStatus {
    if let Some(e) = echo {
        match e.verdicts.lookup(&o.id, &o.id_hash) {
            Some(r) => r.status,
            None => VerdictStatus::Open,
        }
    } else if verdict_is_discharged(o) {
        VerdictStatus::Discharged
    } else {
        VerdictStatus::Open
    }
}

/// Per-module class accounting over all five kinds, in fixed order, from the
/// resolved verdicts. A module without an artifact yields zeros.
fn classes_for(set: Option<&OblSet>, echo: Option<&Echo>) -> Vec<ClassAccounting> {
    let mut classes: Vec<ClassAccounting> =
        KIND_ORDER.iter().map(|k| ClassAccounting::zero(k)).collect();
    let Some(set) = set else { return classes };
    for o in &set.obligations {
        let idx = o.kind.idx();
        if idx >= classes.len() {
            continue;
        }
        let slot = &mut classes[idx];
        slot.total = slot.total.saturating_add(1);
        match resolved_status(o, echo) {
            VerdictStatus::Discharged => slot.discharged += 1,
            VerdictStatus::Assumed => slot.assumed += 1,
            VerdictStatus::Open => slot.open += 1,
        }
    }
    classes
}

/// The descriptor discharge rule fallback (P3): `mmio-bounds` with a constant
/// offset that fits — `off + width ≤ size` — is discharged.
fn verdict_is_discharged(o: &Obligation) -> bool {
    match (&o.formula, o.kind) {
        (Formula::OffsetLE { off, width, size }, Kind::MmioBounds) => {
            off.is_some_and(|off| off.saturating_add(*width) <= *size)
        }
        _ => false,
    }
}

/// Every obligation resolving open lands in the `open` list.
fn collect_open(set: &OblSet, echo: Option<&Echo>, module: &str, out: &mut Vec<OpenObligation>) {
    for o in &set.obligations {
        if resolved_status(o, echo) != VerdictStatus::Open {
            continue;
        }
        out.push(OpenObligation {
            id: o.id.clone(),
            kind: o.kind.as_str().to_string(),
            module: module.to_string(),
            word: o.site.word.clone(),
            site: format!("{}.{}", o.site.word, o.site.occurrence),
            line: o.site.span.line,
        });
    }
}

/// Every obligation resolving assumed lands in the `assumed` list with its
/// recorded justification (P4, FR-15).
fn collect_assumed(set: &OblSet, echo: Option<&Echo>, module: &str, out: &mut Vec<AssumedObligation>) {
    for o in &set.obligations {
        let Some(e) = echo else { continue };
        let Some(r) = e.verdicts.lookup(&o.id, &o.id_hash) else {
            continue;
        };
        if r.status != VerdictStatus::Assumed {
            continue;
        }
        out.push(AssumedObligation {
            id: o.id.clone(),
            kind: o.kind.as_str().to_string(),
            module: module.to_string(),
            word: o.site.word.clone(),
            site: format!("{}.{}", o.site.word, o.site.occurrence),
            line: o.site.span.line,
            justification: r.justification.clone(),
        });
    }
}

/// The aperture-size facts used by discharged `mmio-bounds` obligations, each
/// copied into the trusted list (T2 / FR-15).
fn collect_mmio_sizes(set: &OblSet, module: &str, out: &mut Vec<TrustedAssumption>) {
    for o in &set.obligations {
        if o.kind != Kind::MmioBounds || !verdict_is_discharged(o) {
            continue;
        }
        for a in &o.assumptions {
            if let verifier::model::Assumption::ApertureSize { size, .. } = a {
                out.push(TrustedAssumption {
                    kind: "descriptor".to_string(),
                    what: format!("{module}: mmio aperture size={size}"),
                });
            }
        }
    }
}

/// Sum the per-class accounting across modules (NFR-9 one-liner).
fn report_totals(r: &VerifyReport) -> (u32, u32, u32, u32) {
    let mut total = 0u32;
    let mut discharged = 0u32;
    let mut assumed = 0u32;
    let mut open = 0u32;
    for m in &r.modules {
        for c in &m.classes {
            total = total.saturating_add(c.total);
            discharged = discharged.saturating_add(c.discharged);
            assumed = assumed.saturating_add(c.assumed);
            open = open.saturating_add(c.open);
        }
    }
    (total, discharged, assumed, open)
}

/// Slice P4 policy enforcement (FR-18): `no-open` fails the build with E6410
/// listing every open obligation (module/word/site/line); `no-open-no-
/// assumptions` additionally fails on assumed verdicts. `--verify=off` is
/// exempt — it is the legacy all-checks mode, not a policy decision.
fn enforce_policy(report: &VerifyReport, policy: VerifyPolicy) -> Result<(), TyuError> {
    match policy {
        VerifyPolicy::OpenOk => Ok(()),
        VerifyPolicy::NoOpen | VerifyPolicy::NoOpenNoAssumptions => {
            let fails_assumed = policy == VerifyPolicy::NoOpenNoAssumptions && !report.assumed.is_empty();
            if report.open.is_empty() && !fails_assumed {
                return Ok(());
            }
            if !report.open.is_empty() {
                eprintln!("tyu: error[E6410]: open obligations (--verify-policy={}):", policy.as_str());
                for o in report.open.iter().take(64) {
                    eprintln!(
                        "  {} — {}::{}.{} line {}",
                        o.kind, o.module, o.word, o.site, o.line
                    );
                }
                if report.open.len() > 64 {
                    eprintln!("  … and {} more", report.open.len() - 64);
                }
            }
            if fails_assumed {
                eprintln!(
                    "tyu: error[E6410]: assumed verdicts not allowed (--verify-policy={}):",
                    policy.as_str()
                );
                for a in report.assumed.iter().take(64) {
                    eprintln!(
                        "  {} — {}::{}.{} line {}{}",
                        a.kind,
                        a.module,
                        a.word,
                        a.site,
                        a.line,
                        a.justification
                            .as_ref()
                            .map(|j| format!(" (justification: {j})"))
                            .unwrap_or_default(),
                    );
                }
            }
            Err(TyuError::Build(format!(
                "E6410: {} open obligations and {} assumed under --verify-policy={}",
                report.open.len(),
                if fails_assumed { report.assumed.len() } else { 0 },
                policy.as_str(),
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verifier::model::{ExtractionCtx, Formula, Kind, Oel, Provenance};
    use verifier::verdict::VerdictRecord;

    fn set_with_main(main_high: u32, top: bool) -> (String, Option<OblSet>) {
        use ir::{EffectSet, High, StackBound};
        let mut ctx = ExtractionCtx::new(b"App");
        ctx.begin_word(b"main");
        ctx.push_word_fact(
            b"main",
            StackBound {
                net: 0,
                high: if top { High::Top } else { High::Slots(main_high) },
            },
            EffectSet::empty(),
        );
        ctx.record(
            Kind::SubtypeRange,
            Formula::InRange {
                value: Oel::Var {
                    name: "in.0".to_string(),
                },
                lo: 0,
                hi: 100,
            },
            0,
            0,
            Provenance::Direct,
            Vec::new(),
        );
        ("App".to_string(), Some(ctx.into_set()))
    }

    #[test]
    fn main_context_discharges_when_peak_fits_derived_budget() {
        let sets = vec![set_with_main(3, false)];
        let m = main_context(&sets, Some("App"), Some(16384));
        assert_eq!(m.verdict, "discharged");
        assert_eq!(m.budget, 16384);
        assert_eq!(m.high, 3);
    }

    #[test]
    fn main_context_opens_when_peak_exceeds_budget() {
        let sets = vec![set_with_main(16385, false)];
        let m = main_context(&sets, Some("App"), Some(16384));
        assert_eq!(m.verdict, "open");
    }

    #[test]
    fn main_context_opens_when_geometry_unavailable() {
        // Amended §6.4 fail-closed rule: no derived N_main (missing runtime
        // artifact / symbols) → open, never discharged.
        let sets = vec![set_with_main(3, false)];
        let m = main_context(&sets, Some("App"), None);
        assert_eq!(m.verdict, "open");
        assert_eq!(m.budget, 0);
    }

    #[test]
    fn main_context_opens_on_top() {
        let sets = vec![set_with_main(u32::MAX, true)];
        let m = main_context(&sets, Some("App"), Some(16384));
        assert_eq!(m.verdict, "open");
        assert!(m.top);
    }

    #[test]
    fn echo_resolution_is_authoritative_and_fail_closed() {
        // A discharged record in the echo closes the site; absence opens it;
        // a hash-mismatched record is treated as absence (Q3).
        let set = set_with_main(1, false).1.unwrap();
        let obligation = set.obligations[0].clone();
        let rec = verifier::verdict::VerdictRecord {
            id: obligation.id.clone(),
            id_hash: obligation.id_hash.clone(),
            status: VerdictStatus::Assumed,
            method: None,
            proof_ref: None,
            justification: Some("reviewed".to_string()),
        };
        // Build a minimal echo by encoding + reading back.
        let bytes = verifier::verdict::encode_verdicts(
            "test",
            "0",
            &[rec],
            0,
            &verifier::verdict::EmittedChecksData::default(),
        )
        .unwrap();
        let echo = read_echo(&bytes).unwrap();
        assert_eq!(
            resolved_status(&obligation, Some(&echo)),
            VerdictStatus::Assumed
        );
        // A tampered id_hash fails closed to open.
        let tampered = VerdictRecord {
            id: obligation.id.clone(),
            id_hash: "0000000000000000".to_string(),
            status: VerdictStatus::Discharged,
            method: Some("interval".to_string()),
            proof_ref: None,
            justification: None,
        };
        let bytes = verifier::verdict::encode_verdicts(
            "test",
            "0",
            &[tampered],
            0,
            &verifier::verdict::EmittedChecksData::default(),
        )
        .unwrap();
        let echo = read_echo(&bytes).unwrap();
        assert_eq!(
            resolved_status(&obligation, Some(&echo)),
            VerdictStatus::Open
        );
        // Without an echo, the P3 descriptor rule leaves this site open.
        assert_eq!(resolved_status(&obligation, None), VerdictStatus::Open);
    }
}