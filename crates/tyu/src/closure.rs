//! Image-level assumption closure (§Q7 rule 2, P8.2) — the executable face
//! of the T-CL registry theorem (`Tyu.Sound.AssumptionClosure`, P8.1).
//!
//! A caller-side `contract-pre` certificate is valid only relative to the
//! callee module that was itself proven (BUG-MODEL A1 applied to verdicts).
//! `check_image_closure` walks the artifact `assumptions` edges of every
//! closed (`discharged` with `trust ∈ {proof, checked}`) obligation across
//! the whole image:
//!
//! - a `runtime-check` edge is a terminal (the emitted check IS the
//!   discharge — the T-CL base case);
//! - an `Obligation { id, module }` edge must land on an obligation whose
//!   own closure is sound; otherwise the *dependent* must resolve open with
//!   witness `assumption-unresolved` (never silently reuse a discharge
//!   whose dependency is unproven — T-CL `open_edge_not_well_closed`);
//! - a cycle in the closed-assumption graph is malformed — E6419-class,
//!   fail-closed (T-CL `cyclic_not_well_closed`).
//!
//! The walk is a memoized DFS (an obligation's closure soundness is a pure
//! function of its subgraph), so diamond dependencies are assessed once.
//! The per-image result feeds the report's `closure` section, and the
//! `proven` policy's E6410 enforcement reaches it through the force-opened
//! dependents.

use std::collections::{HashMap, HashSet};
use verifier::model::{AssumptionEdge, OblSet};
use verifier::verdict::{Trust, VerdictStatus};

/// A dependent whose closure failed — must resolve open with witness
/// `assumption-unresolved`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnresolvedDependent {
    pub module: String,
    pub id: String,
    pub kind: String,
    pub word: String,
    pub occurrence: u32,
    pub line: u32,
    /// The direct dependency that failed to close (`<module>::<id>`).
    pub dependency: String,
}

/// The per-image closure outcome (mirrors the report's `closure` section).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageClosure {
    /// The assumption graph is well-closed: no unresolved dependents.
    pub well_closed: bool,
    /// Closed obligations whose assumption graphs were walked.
    pub checked: u32,
    /// Dependents forced open with witness `assumption-unresolved`.
    pub unresolved: Vec<UnresolvedDependent>,
}

/// A cycle in the closed-assumption graph — malformed (E6419-class).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClosureCycle {
    /// The `(module, id)` path that loops back on itself (the cycle).
    pub path: Vec<(String, String)>,
}

/// A failure of the pre-pass-2 harvest adjustment: a genuine graph cycle
/// (E6419-class, fail-closed) or a malformed input document (the build
/// must never guess at a certificate).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HarvestClosureError {
    /// The closed-assumption graph cycles — E6419-class malformed.
    Cycle(ClosureCycle),
    /// A harvested verdicts document is unparseable/invalid.
    Malformed(String),
}

/// The memoized DFS closure walker.
struct Walker {
    // module -> artifact
    sets: HashMap<String, OblSet>,
    // (module, id) of obligations resolved closed (proof/checked)
    closed: HashSet<(String, String)>,
    // cached soundness per obligation (false = closed but unresolvable)
    memo: HashMap<(String, String), bool>,
    // (module, id) on the current DFS path (cycle detection)
    on_path: Vec<(String, String)>,
}

impl Walker {
    /// Assess one obligation's closure soundness. `true` = its closure
    /// terminates on closed/runtime-check obligations (T-CL); `false` =
    /// an unresolved dependency forces it open; `Err(cycle)` = malformed.
    fn assess_sound(&mut self, module: &str, id: &str) -> Result<bool, ClosureCycle> {
        let key = (module.to_string(), id.to_string());
        // Cycle detection: re-entering a node still on the DFS path.
        if let Some(idx) = self.on_path.iter().position(|k| *k == key) {
            let mut path: Vec<(String, String)> = self.on_path[idx..].to_vec();
            path.push(key);
            return Err(ClosureCycle { path });
        }
        if let Some(cached) = self.memo.get(&key) {
            return Ok(*cached);
        }
        // An obligation that is not resolved closed (proof/checked) cannot
        // satisfy a dependency.
        if !self.closed.contains(&key) {
            self.memo.insert(key, false);
            return Ok(false);
        }
        self.on_path.push(key.clone());
        let mut sound = true;
        let assumptions: Vec<AssumptionEdge> = self
            .sets
            .get(module)
            .and_then(|set| set.obligations.iter().find(|o| o.id == id))
            .map(|o| o.assumptions.clone())
            .unwrap_or_default();
        for edge in &assumptions {
            match edge {
                // The emitted check IS the discharge — T-CL base case.
                AssumptionEdge::RuntimeCheck => {}
                AssumptionEdge::Obligation {
                    id: dep_id,
                    module: dep_mod,
                } => {
                    if !self.assess_sound(dep_mod, dep_id)? {
                        sound = false;
                        break;
                    }
                }
            }
        }
        self.on_path.pop();
        self.memo.insert(key, sound);
        Ok(sound)
    }
}

/// Run the image-level closure check over every module's artifact + its
/// resolved (closed-or-assumed) verdict records. `sets`/`records` are
/// parallel per-module lists in build (callee-first) order; a module with
/// no records leaves nothing closed there, so nothing is walked.
///
/// Returns `Err(ClosureCycle)` — E6419-class malformed — when the
/// closed-assumption graph cycles; the build must fail closed, never
/// resolve a cycle.
pub fn check_image_closure_records(
    modules: &[(
        String,
        Option<OblSet>,
        Vec<verifier::verdict::VerdictRecord>,
    )],
) -> Result<ImageClosure, ClosureCycle> {
    let mut walker = Walker {
        sets: HashMap::new(),
        closed: HashSet::new(),
        memo: HashMap::new(),
        on_path: Vec::new(),
    };
    for (name, set, _records) in modules {
        if let Some(s) = set {
            walker.sets.insert(name.clone(), s.clone());
        }
    }
    for (name, set, recs) in modules {
        let Some(set) = set else { continue };
        for o in &set.obligations {
            let closed = recs
                .iter()
                .find(|r| r.id == o.id && r.id_hash == o.id_hash)
                .map(|r| {
                    r.status == VerdictStatus::Discharged
                        && matches!(r.trust, Trust::Proof | Trust::Checked)
                })
                .unwrap_or(false);
            if closed {
                walker.closed.insert((name.clone(), o.id.clone()));
            }
        }
    }

    let mut unresolved: Vec<UnresolvedDependent> = Vec::new();
    let mut checked = 0u32;
    for (name, set, _recs) in modules {
        let Some(set) = set else { continue };
        for o in &set.obligations {
            // Only obligations that carry assumption edges rest on anything.
            if o.assumptions.is_empty() {
                continue;
            }
            if !walker.closed.contains(&(name.clone(), o.id.clone())) {
                continue;
            }
            checked = checked.saturating_add(1);
            // The dependent rests on its DIRECT edges; the witness names the
            // direct target whose own closure failed.
            let mut failed_dep: Option<String> = None;
            for edge in &o.assumptions {
                match edge {
                    AssumptionEdge::RuntimeCheck => {}
                    AssumptionEdge::Obligation {
                        id: dep_id,
                        module: dep_mod,
                    } => {
                        if !walker.assess_sound(dep_mod, dep_id)? {
                            // The edge's canonical id already names the module
                            // (`<module>::<word>::<kind>::<occ>` — §6.1).
                            failed_dep = Some(dep_id.clone());
                            break;
                        }
                    }
                }
            }
            if let Some(dep) = failed_dep {
                unresolved.push(UnresolvedDependent {
                    module: name.clone(),
                    id: o.id.clone(),
                    kind: o.kind.as_str().to_string(),
                    word: o.site.word.clone(),
                    occurrence: o.site.occurrence,
                    line: o.site.span.line,
                    dependency: dep,
                });
            }
        }
    }
    unresolved.sort_by(|a, b| (&a.module, &a.id).cmp(&(&b.module, &b.id)));
    Ok(ImageClosure {
        well_closed: unresolved.is_empty(),
        checked,
        unresolved,
    })
}

/// Run the image-level closure check over every module's artifact + its
/// verdicts echo. The pairing is structural — one tuple per module
/// (`name, artifact, echo`) — so a caller reordering one parallel list can
/// never silently mispair modules.
pub fn check_image_closure(
    modules: &[(String, Option<OblSet>, Option<verifier::verdict::Echo>)],
) -> Result<ImageClosure, ClosureCycle> {
    let records: Vec<(
        String,
        Option<OblSet>,
        Vec<verifier::verdict::VerdictRecord>,
    )> = modules
        .iter()
        .map(|(name, set, echo)| {
            (
                name.clone(),
                set.clone(),
                echo.as_ref()
                    .map(|e| e.verdicts.records.clone())
                    .unwrap_or_default(),
            )
        })
        .collect();
    check_image_closure_records(&records)
}

// ---------------------------------------------------------------------------
// Pre-pass-2 harvest adjustment (P8.2): the closure runs BETWEEN the harvest
// and pass-2 codegen, and the force-opened verdict set is fed to langc — so
// codegen sees the dependents open and RETAINS their checks (the plan's
// shape; never an elided check with an open report row).
// ---------------------------------------------------------------------------

/// Apply the image-level closure to the harvested verdict documents:
/// dependents whose closure fails (a callee module that was not itself
/// proven) are flipped to `open` with witness `assumption-unresolved` in
/// the module's document, so pass-2 langc consumes them open and retains
/// the checks. A cycle is E6419-malformed (the build fails).
///
/// Returns the (possibly adjusted) `(module, v2 document)` pairs; modules
/// with no unresolved dependents are returned byte-unchanged.
pub fn apply_harvest_closure(
    sets: &[(String, Option<OblSet>)],
    docs: &[(String, String)],
) -> Result<Vec<(String, String)>, HarvestClosureError> {
    // Parse the harvested documents (fail-closed: an unparseable harvest
    // document aborts the build — never guess at a certificate).
    let mut parsed: Vec<(String, verifier::verdict::Verdicts)> = Vec::new();
    for (module, text) in docs {
        match verifier::verdict::read_verdicts(text.as_bytes()) {
            Ok(v) => parsed.push((module.clone(), v)),
            Err(e) => {
                return Err(HarvestClosureError::Malformed(format!(
                    "harvest verdicts of {module} invalid (E{}): {e:?}",
                    e.code()
                )));
            }
        }
    }
    // Pair the artifacts and the harvested documents structurally BY NAME —
    // an index-zip of two parallel lists would silently mispair modules if
    // either list were reordered.
    let mut modules: Vec<(
        String,
        Option<OblSet>,
        Vec<verifier::verdict::VerdictRecord>,
    )> = Vec::new();
    for (name, set) in sets {
        let records = parsed
            .iter()
            .find(|(m, _)| m == name)
            .map(|(_, v)| v.records.clone())
            .unwrap_or_default();
        modules.push((name.clone(), set.clone(), records));
    }
    let closure = check_image_closure_records(&modules).map_err(HarvestClosureError::Cycle)?;
    if closure.unresolved.is_empty() {
        return Ok(docs.to_vec());
    }
    let mut rewritten: std::collections::HashMap<String, Vec<verifier::verdict::VerdictRecord>> =
        parsed
            .iter()
            .map(|(m, v)| (m.clone(), v.records.clone()))
            .collect();
    for u in &closure.unresolved {
        let witness = unresolved_witness(&u.dependency);
        let recs = rewritten
            .get_mut(&u.module)
            .expect("unresolved module has a harvested document");
        for r in recs.iter_mut() {
            if r.id == u.id {
                r.status = VerdictStatus::Open;
                r.trust = Trust::Open;
                r.method = None;
                r.statement_hash = None;
                r.proof = None;
                r.surface = None;
                r.authored = None;
                r.witness_reason = Some(witness.clone());
            }
        }
    }
    let mut out: Vec<(String, String)> = Vec::with_capacity(docs.len());
    for (module, v) in &parsed {
        let recs = &rewritten[module];
        match verifier::verdict::encode_verdicts(
            "harvest",
            "0.1.0",
            v.certifier.as_ref(),
            &v.target,
            &v.model_semantics,
            recs,
            0,
            &Default::default(),
        ) {
            Ok(bytes) => out.push((
                module.clone(),
                String::from_utf8(bytes)
                    .map_err(|_| HarvestClosureError::Malformed("non-UTF-8 verdicts".into()))?,
            )),
            Err(e) => {
                return Err(HarvestClosureError::Malformed(format!(
                    "closure-adjusted verdicts encode failed: {e:?}"
                )));
            }
        }
    }
    Ok(out)
}

/// The witness reason a force-opened dependent carries (§Q7 rule 2):
/// `assumption-unresolved: <module>::<id>`.
pub fn unresolved_witness(dep: &str) -> String {
    format!("assumption-unresolved: {dep}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use verifier::model::Kind;
    use verifier::testutil::{
        edge, mk_echo, mk_harvest_doc, mk_obl_set, mk_obligation, mk_synthetic_set,
    };

    /// A single-contract-pre artifact (the closure tests' default shape).
    fn obl(module: &str, id: &str, assumptions: Vec<verifier::model::AssumptionEdge>) -> OblSet {
        mk_obl_set(
            module,
            vec![mk_obligation(
                id,
                "0000000000000000",
                Kind::ContractPre,
                assumptions,
            )],
        )
    }

    /// An echo closing every listed id under the default hash.
    fn echo_closed(ids: &[&str]) -> verifier::verdict::Echo {
        let pairs: Vec<(&str, &str)> = ids.iter().map(|i| (*i, "0000000000000000")).collect();
        mk_echo(&pairs)
    }

    /// A harvested-style v2 document closing the given (id, id_hash) pairs.
    fn harvest_doc(records: &[(String, String)]) -> String {
        let pairs: Vec<(&str, &str)> = records
            .iter()
            .map(|(i, h)| (i.as_str(), h.as_str()))
            .collect();
        mk_harvest_doc(&pairs)
    }

    /// Structural module form (artifact + echo per tuple).
    fn run(
        modules: Vec<(String, Option<OblSet>, Option<verifier::verdict::Echo>)>,
    ) -> Result<ImageClosure, ClosureCycle> {
        check_image_closure(&modules)
    }

    #[test]
    fn callee_closed_closure_is_sound() {
        // App::main::contract-pre::0 → Bank::withdraw::contract-pre::0 (closed).
        let bank = obl("Bank", "Bank::withdraw::contract-pre::0", vec![]);
        let app = obl(
            "App",
            "App::main::contract-pre::0",
            vec![edge("Bank", "Bank::withdraw::contract-pre::0")],
        );
        let c = run(vec![
            (
                "Bank".to_string(),
                Some(bank),
                Some(echo_closed(&["Bank::withdraw::contract-pre::0"])),
            ),
            (
                "App".to_string(),
                Some(app),
                Some(echo_closed(&["App::main::contract-pre::0"])),
            ),
        ])
        .expect("no cycle");
        assert!(c.well_closed, "callee proven ⇒ caller closure sound");
        assert_eq!(c.checked, 1, "one edge-bearing obligation walked");
    }

    #[test]
    fn callee_unproven_forces_dependent_open() {
        let bank = obl("Bank", "Bank::withdraw::contract-pre::0", vec![]);
        let app = obl(
            "App",
            "App::main::contract-pre::0",
            vec![edge("Bank", "Bank::withdraw::contract-pre::0")],
        );
        let c = run(vec![
            ("Bank".to_string(), Some(bank), Some(echo_closed(&[]))), // Bank unproven
            (
                "App".to_string(),
                Some(app),
                Some(echo_closed(&["App::main::contract-pre::0"])),
            ),
        ])
        .expect("no cycle");
        assert!(!c.well_closed);
        let u = &c.unresolved[0];
        assert_eq!(u.module, "App");
        assert_eq!(u.id, "App::main::contract-pre::0");
        assert_eq!(u.dependency, "Bank::withdraw::contract-pre::0");
    }

    #[test]
    fn runtime_check_edge_is_a_terminal() {
        let app = obl(
            "App",
            "App::main::contract-pre::0",
            vec![verifier::model::AssumptionEdge::RuntimeCheck],
        );
        let c = run(vec![(
            "App".to_string(),
            Some(app),
            Some(echo_closed(&["App::main::contract-pre::0"])),
        )])
        .expect("no cycle");
        assert!(
            c.well_closed,
            "runtime-check is the discharge (T-CL base case)"
        );
        assert!(c.unresolved.is_empty());
    }

    #[test]
    fn missing_dependency_is_unresolved() {
        // The edge names a module/obligation nowhere in the image.
        let app = obl(
            "App",
            "App::main::contract-pre::0",
            vec![edge("Ghost", "Ghost::f::contract-pre::0")],
        );
        let c = run(vec![(
            "App".to_string(),
            Some(app),
            Some(echo_closed(&["App::main::contract-pre::0"])),
        )])
        .expect("no cycle");
        assert!(!c.well_closed);
        assert_eq!(c.unresolved[0].dependency, "Ghost::f::contract-pre::0");
    }

    #[test]
    fn closed_with_no_edges_is_not_walked() {
        let bank = obl("Bank", "Bank::withdraw::contract-pre::0", vec![]);
        let c = run(vec![(
            "Bank".to_string(),
            Some(bank),
            Some(echo_closed(&["Bank::withdraw::contract-pre::0"])),
        )])
        .expect("no cycle");
        assert!(c.well_closed);
        assert_eq!(c.checked, 0, "an edge-free obligation isn't walked");
    }

    #[test]
    fn an_edge_cycle_is_malformed() {
        // A → B → A in ONE malformed module artifact (the synthetic malformed
        // case the extractor can never produce; the checker must still fail
        // closed, E6419-class).
        let m = mk_synthetic_set(
            "M",
            &[
                (
                    "M::a::contract-pre::0",
                    "0000000000000000",
                    &["M::b::contract-pre::0"],
                ),
                (
                    "M::b::contract-pre::0",
                    "1111111111111111",
                    &["M::a::contract-pre::0"],
                ),
            ],
        );
        let echo = verifier::testutil::mk_echo(&[
            ("M::a::contract-pre::0", "0000000000000000"),
            ("M::b::contract-pre::0", "1111111111111111"),
        ]);
        let err = run(vec![("M".to_string(), Some(m), Some(echo))])
            .expect_err("a cycle is malformed (E6419)");
        assert!(
            err.path.len() >= 3,
            "the cycle path names the loop: {:?}",
            err.path
        );
        assert_eq!(err.path[0], err.path[err.path.len() - 1], "path closes");
    }

    /// The pre-pass-2 adjustment: a caller certificate whose callee is
    /// unproven is flipped to open with the assumption-unresolved witness
    /// BEFORE the document reaches codegen.
    #[test]
    fn apply_harvest_closure_flips_unresolved_dependents() {
        let bank = obl("Bank", "Bank::withdraw::contract-pre::0", vec![]);
        let app = obl(
            "App",
            "App::main::contract-pre::0",
            vec![edge("Bank", "Bank::withdraw::contract-pre::0")],
        );
        let sets = vec![
            ("Bank".to_string(), Some(bank)),
            ("App".to_string(), Some(app)),
        ];
        // App's certificate harvested; Bank's own contract-pre unproven (no
        // record in its document).
        let docs = vec![
            ("Bank".to_string(), harvest_doc(&[])),
            (
                "App".to_string(),
                harvest_doc(&[(
                    "App::main::contract-pre::0".into(),
                    "0000000000000000".into(),
                )]),
            ),
        ];
        let adjusted = apply_harvest_closure(&sets, &docs).expect("no cycle");
        // Bank's document is byte-unchanged; App's is rewritten with the
        // dependent flipped to open + the witness.
        assert_eq!(adjusted[0].1, docs[0].1, "unaffected module byte-unchanged");
        let app_doc = verifier::verdict::read_verdicts(adjusted[1].1.as_bytes()).unwrap();
        let dep = app_doc
            .records
            .iter()
            .find(|r| r.id == "App::main::contract-pre::0")
            .expect("dependent record present");
        assert_eq!(dep.status, VerdictStatus::Open);
        assert_eq!(dep.trust, Trust::Open);
        assert_eq!(dep.method, None);
        assert_eq!(
            dep.proof, None,
            "a flipped certificate is no longer a proof"
        );
        assert_eq!(
            dep.witness_reason.as_deref(),
            Some("assumption-unresolved: Bank::withdraw::contract-pre::0")
        );
    }

    /// The pre-pass-2 adjustment leaves a well-closed harvest byte-unchanged.
    #[test]
    fn apply_harvest_closure_leaves_sound_harvest_unchanged() {
        let bank = obl("Bank", "Bank::withdraw::contract-pre::0", vec![]);
        let app = obl(
            "App",
            "App::main::contract-pre::0",
            vec![edge("Bank", "Bank::withdraw::contract-pre::0")],
        );
        let sets = vec![
            ("Bank".to_string(), Some(bank)),
            ("App".to_string(), Some(app)),
        ];
        let docs = vec![
            (
                "Bank".to_string(),
                harvest_doc(&[(
                    "Bank::withdraw::contract-pre::0".into(),
                    "0000000000000000".into(),
                )]),
            ),
            (
                "App".to_string(),
                harvest_doc(&[(
                    "App::main::contract-pre::0".into(),
                    "0000000000000000".into(),
                )]),
            ),
        ];
        let adjusted = apply_harvest_closure(&sets, &docs).expect("no cycle");
        assert_eq!(adjusted, docs, "a sound harvest passes through untouched");
    }
}
