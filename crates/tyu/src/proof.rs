//! Developer-proof pipeline (PLAN-VERIFY-3 P6) — the generated lake package.
//!
//! P6.1 (`tyu proof init` + package generation + Gen-digest check + locking +
//! caching) and P6.2 (`tyu build --verify-tool=lean` orchestration: gen →
//! digest → lake build, with an honest unproven-statement accounting in the
//! report — no path claims a proof verdict before the P7 harvest exists).
//!
//! Layout (developer-proof-pipeline.md §Q5):
//!
//! ```text
//! <project>/
//!   proofs/                  DEVELOPER-OWNED, VCS-tracked
//!     proofs.lean            root import (scaffolded once by tyu proof init)
//!     Bank.lean              per-module theorems: obl_<name> := by …
//!   .tyu-verify/lean/        GENERATED, content-addressed, git-ignored
//!     lean-toolchain         copied from the port pin (hash-pinned)
//!     lakefile.toml          GENERATED — Tyu + Gen + TyuProofs libs + the
//!                            harvest exe stub
//!     Tyu/                   vendored port library (semantics + interfaces)
//!     Gen/<Module>.lean      GENERATED statements (def stmt_… : Prop)
//!     Gen/<Module>.gen.json  statement metadata (name + hash + omission)
//!     Harvest.lean           exe stub — the P7 harvest entry point
//!     .tyu-gen.json          package-generation state (fingerprint)
//! ```
//!
//! Determinism (FR-16): regeneration is byte-deterministic; unchanged files
//! are not rewritten (mtime-stability keeps lake incremental). Error codes:
//! `E6416` (toolchain / lake / package-generation failure class) and `E6418`
//! (Gen digest mismatch — the pre-lake tamper gate).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use verifier::report::{ModuleStatements, ProofStatus};
use verifier::stmt::{sha256_hex16, StatementContext};

use crate::args::VerifyTool;
use crate::cache;
use crate::error::TyuError;

/// The developer-owned proofs directory name (project root, VCS-tracked).
pub const PROOFS_DIR: &str = "proofs";

/// The generated verification tree (project root, git-ignored).
pub const VERIFY_DIR: &str = ".tyu-verify";

/// The generated Lean package directory under `VERIFY_DIR`.
pub const LEAN_PKG_DIR: &str = "lean";

/// The in-tree Lean port location (relative to the workspace root).
pub const PORT_DIR_REL: &str = "verification/ports/lean";

/// Env override to skip the port build entirely (CI tiers without the Lean
/// toolchain): the proof pipeline is recorded as `lean-skipped`, every
/// statement stays unproven, and the build is not failed by the absence.
pub const SKIP_PORT_BUILD_ENV: &str = "TYU_SKIP_PORT_BUILD";

/// Env override for the package lock wait (tests shorten it).
pub const LOCK_TIMEOUT_ENV: &str = "TYU_VERIFY_LOCK_TIMEOUT_MS";

/// Default lock wait (§7.2: contention waits ≤ 60 s then E6416).
const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(60);

const PKG_STATE_SCHEMA: &str = "tyu.pkg/1";

// ---------------------------------------------------------------------------
// §7.2 verdicts-cache key extension
// ---------------------------------------------------------------------------

/// The verification-environment components of the §7.2 verdicts-slot key.
///
/// Every verdicts-cache slot is named
/// `<Module>-<inputs_fp>-sem-<semantics>-stmt-<stmt>-tc-<toolchain_hash16>-
/// model-<model_id>-proofs-<proof_files_hash16>.verdicts.json` (slashes
/// sanitized to `_` — ids like `tyu.ir-sem/1.0` or `tyu.model/…/1` are not
/// file-name characters). The extended key means a proof-file edit
/// (`proof_files_hash`), a toolchain pin change (`toolchain_hash`), a model
/// change (`model_id`), or a schema/version bump invalidates cached verdicts
/// exactly when it must — P7's *harvested* (`proof`-class) verdicts land in
/// these slots, and a stale slot for a changed proof environment would
/// otherwise be silently reused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyEnvKey {
    semantics: String,
    stmt: String,
    /// FNV-1a of the port's `lean-toolchain` pin bytes (a cache key, not an
    /// integrity digest — FR-14 governs the codec digests).
    toolchain_hash: u64,
    /// The bundle's model-semantics identity (sanitized into the name);
    /// `"unmodeled"` until P12 supplies `platform.toml [model]`.
    model_id: String,
    proof_files_hash: u64,
}

impl VerifyEnvKey {
    /// Build the key for a build: the toolchain pin is read from the port
    /// (absent port → the empty-pin hash, so a repo without the verification
    /// tree still gets a stable, well-defined key), the model id is caller-
    /// supplied, and the proof-files hash covers the developer-owned
    /// `proofs/` directory. Total: never fails.
    pub(crate) fn compute(project_root: &Path, model_id: &str) -> Self {
        let pin = crate::platform::workspace_root()
            .join(PORT_DIR_REL)
            .join("lean-toolchain");
        let pin_bytes = fs::read(&pin).unwrap_or_default();
        Self {
            semantics: verifier::semantics::SEMANTICS_VERSION.to_string(),
            stmt: verifier::stmt::STMT_SCHEMA.to_string(),
            toolchain_hash: cache::fnv1a_u64(&pin_bytes),
            model_id: model_id.to_string(),
            proof_files_hash: proof_files_hash(&project_root.join(PROOFS_DIR)),
        }
    }

    /// The build-wide env key: project root from the CWD resolution, model id
    /// `"unmodeled"` until P12 wires `platform.toml [model]` (the §Q15 honest
    /// default for a bundle without model semantics).
    pub(crate) fn for_build() -> Result<Self, TyuError> {
        let root = project_root_for(None)?;
        Ok(Self::compute(&root, verifier::model::MODEL_UNMODELED))
    }

    /// The sanitized filename slug: `-sem-…-stmt-…-tc-…-model-…-proofs-…`
    /// (slashes → `_`; the dash-sentinel `-sem-` cannot occur in a module
    /// name, which makes the slot name parse unambiguous).
    fn slug(&self) -> String {
        format!(
            "-sem-{}-stmt-{}-tc-{:016x}-model-{}-proofs-{:016x}",
            sanitize_component(&self.semantics),
            sanitize_component(&self.stmt),
            self.toolchain_hash,
            sanitize_component(&self.model_id),
            self.proof_files_hash
        )
    }
}

/// The full slot file name for `(module, inputs_fp)` under an environment.
pub(crate) fn verdicts_slot_name(module: &str, inputs_fp: u64, env: &VerifyEnvKey) -> String {
    format!("{}-{:016x}{}.verdicts.json", module, inputs_fp, env.slug())
}

/// Strict parse of a slot name under the *current* environment: returns
/// `(module, inputs_fp)` only when the name carries exactly this build's
/// slug. A slot from a different proof environment does not parse (it is not
/// this build's slot). Used by the unit tests and available to P7's harvest
/// caching — the lib build sees it as unused until then.
#[allow(dead_code)]
pub(crate) fn parse_verdicts_slot_name(name: &str, env: &VerifyEnvKey) -> Option<(String, u64)> {
    let stem = name.strip_suffix(".verdicts.json")?;
    let core = stem.strip_suffix(&env.slug())?;
    let (module, fp) = core.rsplit_once('-')?;
    if fp.len() != 16 || !fp.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((module.to_string(), u64::from_str_radix(fp, 16).ok()?))
}

/// Loose parse of any verdicts-slot name (`<Module>-<fp:016x>[<slug>]`):
/// module names never contain `-`, so the first two `-`-separated segments
/// are `(module, fp)`. Used by the out-dir pruner to identify stale slots
/// regardless of the env-slug they were written under.
pub(crate) fn parse_any_verdicts_name(name: &str) -> Option<(String, u64)> {
    let stem = name.strip_suffix(".verdicts.json")?;
    let mut it = stem.splitn(3, '-');
    let module = it.next()?;
    let fp = it.next()?;
    if fp.len() != 16 || !fp.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((module.to_string(), u64::from_str_radix(fp, 16).ok()?))
}

/// Sanitize a filename component: `/` (path separators in ids) → `_`; other
/// printable characters pass through untouched. Deterministic.
fn sanitize_component(s: &str) -> String {
    s.chars().map(|c| if c == '/' { '_' } else { c }).collect()
}

// ---------------------------------------------------------------------------
// Project-root resolution
// ---------------------------------------------------------------------------

/// Resolve the developer-project root: `dir` if given, else the `tyu.toml`
/// manifest directory (walking up from the CWD), else the CWD. The generated
/// `.tyu-verify/lean/` package and the developer-owned `proofs/` live here.
pub fn project_root_for(dir: Option<&Path>) -> Result<PathBuf, TyuError> {
    let cwd = std::env::current_dir().map_err(TyuError::Io)?;
    let start = match dir {
        Some(d) if d.is_absolute() => d.to_path_buf(),
        Some(d) => cwd.join(d),
        None => cwd,
    };
    Ok(match crate::project::find_manifest(&start) {
        Some(manifest) => manifest.parent().map(|p| p.to_path_buf()).unwrap_or(start),
        None => start,
    })
}

/// The generated Lean package root for a project: `<root>/.tyu-verify/lean/`.
fn lean_package_path(project_root: &Path) -> PathBuf {
    project_root.join(VERIFY_DIR).join(LEAN_PKG_DIR)
}

// ---------------------------------------------------------------------------
// `tyu proof init` (FR-6): scaffold the developer-owned proofs/ directory.
// ---------------------------------------------------------------------------

/// Scaffold `proofs/` in `root`: the root import, one template per module of
/// the entry `input` (or per `[project]` module), a README, and `.gitignore`
/// entries. Idempotent: existing files are never overwritten.
pub fn proof_init(root: &Path, input: Option<&Path>) -> Result<(), TyuError> {
    let proofs = root.join(PROOFS_DIR);
    fs::create_dir_all(&proofs).map_err(TyuError::Io)?;

    let modules = if let Some(input) = input {
        let graph = crate::graph::resolve_graph(input, &[], Some(root))?;
        graph
            .iter()
            .map(|m| m.name.clone())
            .collect::<Vec<String>>()
    } else {
        project_main_and_modules(root)
    };
    // Deduplicate + sort for deterministic scaffolding.
    let modules: BTreeSet<String> = modules.into_iter().collect();
    let modules: Vec<String> = modules.into_iter().collect();

    // Per-module templates: `import Gen.<Module>` + a doc header. Never
    // clobbers a developer-authored file.
    for module in &modules {
        let path = proofs.join(format!("{module}.lean"));
        if path.exists() {
            continue;
        }
        write_if_changed(&path, per_module_template(module).as_bytes())?;
    }

    // Root import: never clobbers; on first scaffold it imports the module
    // templates just planted; later runs leave it alone (developer-owned).
    let root_path = proofs.join("proofs.lean");
    if !root_path.exists() {
        // Imports MUST precede every other command in Lean — the module doc
        // comment comes after the imports.
        let mut text = String::new();
        for module in &modules {
            text.push_str("import ");
            text.push_str(module);
            text.push('\n');
        }
        text.push('\n');
        text.push_str("/-! Developer proofs root (PLAN-VERIFY-3 P6).\n");
        text.push('\n');
        text.push_str("Import the per-module proof files. `tyu proof init` scaffolds this\n");
        text.push_str("once; add an `import <Module>` line when you create\n");
        text.push_str("`proofs/<Module>.lean` by hand. The generated statements you prove\n");
        text.push_str("live in `.tyu-verify/lean/Gen/<Module>.lean` (regenerated every\n");
        text.push_str("`tyu build --verify-tool=lean`; the `def stmt_… : Prop` definitions\n");
        text.push_str("are the claims). -/\n");
        write_if_changed(&root_path, text.as_bytes())?;
    }

    // README (only on first scaffold).
    let readme = proofs.join("README.md");
    if !readme.exists() {
        let body = "This directory is VCS-tracked and developer-owned.\n\
                    \n\
                    The generated statements you prove are regenerated every\n\
                    `tyu build --verify-tool=lean` into\n\
                    `.tyu-verify/lean/Gen/<Module>.lean` (git-ignored) — the\n\
                    `def stmt_… : Prop` definitions are the claims. Prove each\n\
                    with a theorem of the statement's type in the matching\n\
                    `proofs/<Module>.lean` file.\n\
                    \n\
                    ## Two proof surfaces\n\
                    \n\
                    Every obligation has ONE theorem name (`obl_…`) and one\n\
                    theorem type per build, rendered on one of two surfaces:\n\
                    \n\
                    - **IR surface** (default): `stmt_…` forms over\n\
                      `Tyu.Gen.Stmt`, the concrete IR semantics — meaningful for\n\
                      every word the renderer can express, casts and calls\n\
                      included.\n\
                    - **Source surface**: `src_stmt_…` forms over\n\
                      `Tyu/Src.lean`, the pure-fragment embedding — rendered\n\
                      INSTEAD when the word qualifies. A word qualifies iff\n\
                      its `--emit=ir` text parses entirely as fragment ops\n\
                      (`Tyu.Src.Op`): `const`/`dup`/`drop`/`swap`;\n\
                      `add`/`sub`/`mul`/`cmp_*`/`and`/`or`/`not`;\n\
                      `load`/`store`/`vol_load`/`vol_store`;\n\
                      `local_get`/`local_set`; `br`/`br_if`/`ret`.\n\
                      Casts (`1 as Percent`), calls, and address ops are NOT\n\
                      fragment ops — a word containing any of them stays on\n\
                      the IR surface (the honest §Q2 shrink). Check by\n\
                      reading the word's `--emit=ir` text; the renderer\n\
                      chooses the source surface automatically when every op\n\
                      is in the fragment list above.\n\
                    \n\
                    Because an obligation has one theorem name, you cannot\n\
                    hold an IR-surface and a source-surface proof of the same\n\
                    obligation at once; migrating surfaces means renaming the\n\
                    theorem (the rendered surface is per-build and hash-bound,\n\
                    so a source certificate discharges the IR obligation only\n\
                    through the T-S theorem, and only for a fragment word).\n\
                    \n\
                    Run `tyu proof init` again to re-scaffold — it is idempotent\n\
                    and never overwrites your files. `tyu proof fill` (P10) writes\n\
                    unreviewed candidate proofs to `proofs/candidates/`.\n";
        write_if_changed(&readme, body.as_bytes())?;
    }

    ensure_gitignore(root)?;
    Ok(())
}

/// A per-module proof template: imports the generated statements (`Gen.<M>`)
/// and carries a doc header pointing at the statement surface.
fn per_module_template(module: &str) -> String {
    format!(
        "import Gen.{module}\n\
         \n\
         /-! Developer proofs for module {module}.\n\
         \n\
         The generated statements for this word live in the\n\
         `Tyu.Gen.Corpus.{module}` namespace of\n\
         `.tyu-verify/lean/Gen/{module}.lean` (regenerated every\n\
         `tyu build --verify-tool=lean`). Prove each `stmt_…` obligation with a\n\
         theorem of the same type, e.g.:\n\
         \n\
         \x20 theorem obl_… : Tyu.Gen.Corpus.{module}.stmt_… := by …\n\
         -/\n\
         namespace Tyu.Gen.Corpus.{module}\n\
         end Tyu.Gen.Corpus.{module}\n",
    )
}

/// `[project]` main + modules from `tyu.toml`, when present.
fn project_main_and_modules(root: &Path) -> Vec<String> {
    let manifest_path = root.join("tyu.toml");
    let Ok(text) = fs::read_to_string(&manifest_path) else {
        return Vec::new();
    };
    let Ok(manifest) = toml::from_str::<crate::project::ProjectManifest>(&text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(main) = &manifest.project.main {
        if let Some(name) = module_name_from_path(main) {
            out.push(name);
        }
    }
    for path in &manifest.project.modules {
        if let Some(name) = module_name_from_path(path) {
            out.push(name);
        }
    }
    out
}

/// `<dir>/<snake_case>.mod` → `<ModuleName>` (the module *declaration* name
/// is unknowable without parsing; the file-stem PascalCase form is the
/// conventional match `expected_object_path` uses).
fn module_name_from_path(path: &str) -> Option<String> {
    let stem = Path::new(path).file_stem()?.to_str()?;
    let name: String = stem
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect();
    (!name.is_empty()).then_some(name)
}

/// Ensure `root/.gitignore` covers the generated verification tree; appends
/// only missing lines, never touches existing content.
fn ensure_gitignore(root: &Path) -> Result<(), TyuError> {
    let ignore = root.join(".gitignore");
    let existing = fs::read_to_string(&ignore).unwrap_or_default();
    let mut text = existing.clone();
    for (line, _) in [(".tyu-verify/\n", ".tyu-verify/"), ("target/\n", "target/")] {
        if !existing.lines().any(|l| l.trim() == line.trim()) {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(line);
        }
    }
    if text != existing {
        write_if_changed(&ignore, text.as_bytes())?;
    }
    Ok(())
}

/// `tyu proof fill` (PLAN-VERIFY-3 P10.2): candidate-proof generation.
///
/// Extracts the project's obligation artifacts (`langc --emit=obligations`),
/// renders the Gen statements (the port's `gen` renderer), and runs the
/// port's `fill` exe to write **unreviewed candidate theorem files** under
/// `proofs/candidates/<id>.lean`, each headed by the machine-readable marker
/// `-- tyu:candidate obligation=<id>` (§Q10). Developer-authored files are
/// never modified: candidates land only in `proofs/candidates/` (created
/// here) and are excluded from the harvest's *attribution* accounting only
/// in the sense that a harvested candidate verdict is `authored:
/// "candidate"` — its `trust: proof` is still kernel-gated.
///
/// Returns the candidate count written.
pub fn proof_fill(
    root: &Path,
    input: Option<&Path>,
    include_dirs: &[PathBuf],
    sysroot: Option<&Path>,
    fill_budget: Option<u32>,
    target: codegen_core::Target,
) -> Result<u32, TyuError> {
    let port_dir = crate::platform::workspace_root().join(PORT_DIR_REL);
    if !port_dir.join("lean-toolchain").is_file() {
        return Err(TyuError::Build(format!(
            "E6416: Lean verification port missing at '{}' (expected a \
             verification/ports/lean package with lean-toolchain)",
            port_dir.display()
        )));
    }

    // Prefer the latest build's extracted artifacts (`.tyu-oblig/` under the
    // project out dir, for THIS target); otherwise extract fresh from the
    // given input.
    let artifacts = find_extracted_artifacts(root, target)?;
    let artifacts = if artifacts.is_empty() {
        let entry = input.ok_or_else(|| {
            TyuError::Build(
                "E6416: no obligation artifacts found (run `tyu build \
                 --verify-tool=lean` first, or pass an input .mod to \
                 `tyu proof fill`)"
                    .into(),
            )
        })?;
        extract_artifacts_for_fill(root, entry, include_dirs, sysroot, target)?
    } else {
        artifacts
    };
    if artifacts.is_empty() {
        return Err(TyuError::Build(
            "E6416: no obligation artifacts to fill (nothing falls into the \
             statement pipeline)"
                .into(),
        ));
    }

    // Render the Gen statements (the `gen` renderer) into a temp dir so the
    // `fill` exe has the `tyu.gen/1` metadata beside the artifacts.
    let gen_tmp = std::env::temp_dir().join(format!("tyu-fill-gen-{}", std::process::id()));
    let _ = fs::remove_dir_all(&gen_tmp);
    let gen_bin = ensure_gen_exe(&port_dir)?;
    let mut cmd = Command::new(&gen_bin);
    cmd.arg("--render").arg("--obl");
    for a in &artifacts {
        cmd.arg(a);
    }
    cmd.arg("--out").arg(&gen_tmp);
    let out = cmd.output().map_err(|e| {
        TyuError::Build(format!(
            "E6416: running gen renderer '{}': {e}",
            gen_bin.display()
        ))
    })?;
    if !out.status.success() {
        let _ = fs::remove_dir_all(&gen_tmp);
        return Err(TyuError::Build(
            "E6416: gen renderer failed while preparing `tyu proof fill`".into(),
        ));
    }
    let gen_metas = gen_metadata_files(&gen_tmp);

    // Run the port `fill` exe: write candidate files into `proofs/candidates/`.
    let candidates_dir = root.join(PROOFS_DIR).join("candidates");
    fs::create_dir_all(&candidates_dir).map_err(TyuError::Io)?;
    let fill_bin = ensure_fill_exe(&port_dir)?;
    let mut cmd = Command::new(&fill_bin);
    for a in &artifacts {
        cmd.arg(format!("--obl={}", a.display()));
    }
    for m in &gen_metas {
        cmd.arg(format!("--meta={}", m.display()));
    }
    cmd.arg(format!("--out={}", candidates_dir.display()));
    cmd.arg(format!("--fill-budget={}", fill_budget.unwrap_or(10)));
    let out = cmd.output().map_err(|e| {
        TyuError::Build(format!(
            "E6416: running fill exe '{}': {e}",
            fill_bin.display()
        ))
    })?;
    let _ = fs::remove_dir_all(&gen_tmp);
    if !out.status.success() {
        return Err(TyuError::Build(format!(
            "E6416: fill failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let written = candidate_files(&candidates_dir).len() as u32;
    eprintln!(
        "tyu: proof fill: {} candidate file(s) written to '{}' (unreviewed; \
         review by removing the `-- tyu:candidate obligation=` marker line)",
        written,
        candidates_dir.display()
    );
    Ok(written)
}

/// Artifacts already extracted by the last build: `<root>/target/tyu/<triple>/
/// .tyu-oblig/*.obl.json` (the pass-1 extraction location) — for the fill's
/// target ONLY (statements are `(triple, model)`-relative, §Q3; mixing
/// targets would fill a module twice).
fn find_extracted_artifacts(
    root: &Path,
    target: codegen_core::Target,
) -> Result<Vec<PathBuf>, TyuError> {
    let mut out = Vec::new();
    let triple_name = String::from_utf8_lossy(target.triple()).into_owned();
    let base = root.join("target").join("tyu").join(&triple_name);
    if let Ok(rd) = fs::read_dir(&base) {
        for e in rd.flatten() {
            if e.path().extension().and_then(|x| x.to_str()) == Some("obl.json") {
                out.push(e.path());
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Extract the obligation artifacts for an entry `.mod` into a temp dir
/// (`langc --emit=obligations` over the resolved module graph — the same
/// pass-1 invocation the build uses). Returns the artifact paths (one per
/// module, sorted). This is the standalone fill source: `tyu proof fill
/// entry.mod` works without a prior build.
fn extract_artifacts_for_fill(
    root: &Path,
    input: &Path,
    include_dirs: &[PathBuf],
    sysroot: Option<&Path>,
    target: codegen_core::Target,
) -> Result<Vec<PathBuf>, TyuError> {
    let langc = crate::toolchain::resolve_tool("langc")?;
    let triple = std::str::from_utf8(target.triple())
        .map_err(|_| TyuError::Build("non-UTF-8 target triple".into()))?;
    let modules = crate::graph::resolve_graph(input, include_dirs, sysroot)?;
    if modules.is_empty() {
        return Err(TyuError::Build(
            "E6416: `tyu proof fill` resolved no modules from the input".into(),
        ));
    }
    let dir = std::env::temp_dir().join(format!("tyu-fill-extract-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(TyuError::Io)?;
    for module in &modules {
        let mut cmd = Command::new(&langc);
        cmd.arg("--emit=obligations");
        cmd.arg(format!("--out-dir={}", dir.display()));
        cmd.arg(format!("--target={}", triple));
        if let Some(sr) = sysroot {
            cmd.arg(format!("--sysroot={}", sr.display()));
        }
        for inc in include_dirs {
            cmd.arg("-I");
            cmd.arg(inc);
        }
        cmd.arg(&module.path);
        let out = cmd.output().map_err(|e| {
            TyuError::Build(format!(
                "E6416: running langc (`--emit=obligations`) for '{}': {e}",
                module.path.display()
            ))
        })?;
        if !out.status.success() {
            let _ = fs::remove_dir_all(&dir);
            return Err(TyuError::Build(format!(
                "E6416: langc failed extracting obligations for '{}':\n{}",
                module.path.display(),
                String::from_utf8_lossy(&out.stderr)
            )));
        }
    }
    let mut artifacts: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".obl.json"))
            {
                artifacts.push(p);
            }
        }
    }
    artifacts.sort();
    if artifacts.is_empty() {
        let _ = fs::remove_dir_all(&dir);
        return Err(TyuError::Build(
            "E6416: langc emitted no obligation artifacts for the input".into(),
        ));
    }
    let _ = root;
    Ok(artifacts)
}

/// The `<Module>.gen.json` metadata files in a rendered Gen dir (sorted).
fn gen_metadata_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("json") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The candidate files under `proofs/candidates/` (sorted).
fn candidate_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("lean") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// The candidate-authored obligation ids under `proofs/candidates/`: the
/// `-- tyu:candidate obligation=<id>` marker lines, in file order (the
/// harvest's attribution set). Empty when no candidates exist.
pub fn candidate_ids(project_root: &Path) -> Vec<String> {
    let dir = project_root.join(PROOFS_DIR).join("candidates");
    let mut out = Vec::new();
    for f in candidate_files(&dir) {
        let Ok(text) = fs::read_to_string(&f) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("-- tyu:candidate obligation=") {
                let id = rest.trim();
                if !id.is_empty() {
                    out.push(id.to_string());
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Locate (building if necessary) the port's `fill` executable.
fn ensure_fill_exe(port_dir: &Path) -> Result<PathBuf, TyuError> {
    let bin = port_dir
        .join(".lake")
        .join("build")
        .join("bin")
        .join("fill");
    if bin.is_file() {
        return Ok(bin);
    }
    eprintln!(
        "tyu: building the port's fill exe in '{}'",
        port_dir.display()
    );
    let status = Command::new("lake")
        .current_dir(port_dir)
        .args(["build", "fill"])
        .status()
        .map_err(|e| {
            TyuError::Build(format!(
                "E6416: spawning `lake build fill` in '{}': {e}",
                port_dir.display()
            ))
        })?;
    if !status.success() {
        return Err(TyuError::Build(
            "E6416: `lake build fill` failed in the port".into(),
        ));
    }
    if !bin.is_file() {
        return Err(TyuError::Build(format!(
            "E6416: fill exe not produced in '{}'",
            port_dir.display()
        )));
    }
    Ok(bin)
}

// ---------------------------------------------------------------------------
// P6.2 entry: the full `--verify-tool=lean` pipeline
// ---------------------------------------------------------------------------

/// Run the developer-proof pipeline for a build: package generation (P6.1) →
/// Gen-digest verification (E6418) → the elaborating `lake build` → the
/// harvest (P7.1: kernel-checked theorems become `tyu.verdicts/v2`) → an
/// honest per-module statement accounting. A theorem is worth exactly what
/// the kernel and the axiom audit say it is.
///
/// Returns the [`ProofStatus`] AND the per-module harvest v2 verdict files
/// (absolute paths) the caller feeds to pass-2 codegen (FR-3).
pub fn run_lean_pipeline(
    tool: VerifyTool,
    module_obligations: &[(String, PathBuf)],
) -> Result<(ProofStatus, Vec<(String, PathBuf)>), TyuError> {
    debug_assert_eq!(tool, VerifyTool::Lean);
    // P6 test tier: without a Lean toolchain (CI), the port build can be
    // skipped explicitly — the report then records `lean-skipped` and every
    // statement unproven. This is an *opt-out*, not a silent failure mode.
    if std::env::var_os(SKIP_PORT_BUILD_ENV).is_some() {
        eprintln!(
            "tyu: --verify-tool=lean: port build skipped ({}), every statement remains unproven",
            SKIP_PORT_BUILD_ENV
        );
        return Ok((ProofStatus::skipped(), Vec::new()));
    }

    let project_root = project_root_for(None)?;
    let port_dir = crate::platform::workspace_root().join(PORT_DIR_REL);
    if !port_dir.join("lean-toolchain").is_file() {
        return Err(TyuError::Build(format!(
            "E6416: Lean verification port missing at '{}' (expected a \
             verification/ports/lean package with lean-toolchain)",
            port_dir.display()
        )));
    }

    // Toolchain pin + present-and-matching check (§7.2, E6416 fail-closed).
    let pin = fs::read_to_string(port_dir.join("lean-toolchain"))
        .map_err(TyuError::Io)?
        .trim()
        .to_string();
    ensure_lean_toolchain(&pin)?;

    // Group the re-homed obligation artifacts into absolute paths.
    let artifacts: Vec<PathBuf> = module_obligations
        .iter()
        .map(|(_, path)| path.clone())
        .collect();
    if artifacts.is_empty() {
        eprintln!("tyu: --verify-tool=lean: no obligation artifacts to render");
        return Ok((ProofStatus::lean("skipped", "", Vec::new()), Vec::new()));
    }

    let package = generate_package(&project_root, &port_dir, &artifacts)?;

    // The vendored semantics surface must be exactly the port's (a tampered
    // `Tyu/*.lean` in the package would prove the *wrong system* — the
    // T-F2-class risk, closed fail-closed). The SHA-256 over the vendored
    // files as committed is recorded in the report (digest-recorded, §Q5).
    let vendor_digest = verify_vendored_files(&package, &port_dir)?;

    // E6418 pre-lake gate: recompute canonical statement hashes and compare
    // against the rendered Gen surface.
    let mut statements = verify_gen_digests(&package, &artifacts)?;

    // The elaborating build (late the developer-visible gate: their proofs
    // must compile against the generated statements; then the harvest's env
    // is the built package).
    run_lake_build(&package)?;

    // P7.1: the harvest — kernel-checked theorems become v2 verdicts (plus
    // the per-module axiom-audit evidence, `tyu.axiom-audit/1`).
    let harvest = run_harvest(&package, &artifacts)?;
    let doc_pairs: Vec<(String, String)> = harvest
        .iter()
        .map(|(m, v2, _)| (m.clone(), v2.clone()))
        .collect();

    // P8.2: the image-level assumption closure runs BETWEEN the harvest and
    // pass-2 codegen (the plan's shape — §Q7 rule 3). A caller-side
    // `contract-pre` certificate is valid only relative to the callee module
    // that was itself proven; a dependent whose closure fails is flipped to
    // `open` with witness `assumption-unresolved` in its document, so
    // pass-2 langc consumes it open and RETAINS the check — never an elided
    // check with an open report row. A cycle is E6419-malformed (fail-closed).
    let mut closure_sets: Vec<(String, Option<verifier::model::OblSet>)> = Vec::new();
    for ob in &artifacts {
        let bytes = fs::read(ob).map_err(TyuError::Io)?;
        match verifier::codec::read_obl(&bytes) {
            Ok(set) => closure_sets.push((set.module.clone(), Some(set))),
            Err(e) => {
                return Err(TyuError::Build(format!(
                    "E6419: obligation artifact '{}' invalid for closure (E{}): {e:?}",
                    ob.display(),
                    e.code()
                )));
            }
        }
    }
    let adjusted = match crate::closure::apply_harvest_closure(&closure_sets, &doc_pairs) {
        Ok(docs) => docs,
        Err(crate::closure::HarvestClosureError::Cycle(cycle)) => {
            let loop_text = cycle
                .path
                .iter()
                .map(|(m, id)| format!("{m}::{id}"))
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(TyuError::Build(format!(
                "E6419: assumption-closure cycle (malformed) in harvested verdicts: {loop_text}"
            )));
        }
        Err(crate::closure::HarvestClosureError::Malformed(detail)) => {
            return Err(TyuError::Build(format!(
                "E6416: harvest closure input malformed: {detail}"
            )));
        }
    };
    let harvest_out = project_root.join(VERIFY_DIR).join("harvest");
    fs::create_dir_all(&harvest_out).map_err(TyuError::Io)?;
    let mut verdict_files: Vec<(String, PathBuf)> = Vec::new();
    for (module, v2) in adjusted {
        let path = harvest_out.join(format!("{module}.verdicts.v2.json"));
        fs::create_dir_all(harvest_out.parent().unwrap_or(&harvest_out)).ok();
        fs::write(&path, &v2).map_err(TyuError::Io)?;
        // The axiom-audit evidence travels with the verdicts (§Q11 item 2):
        // every harvested theorem with its transitive axiom set, next to
        // the permitted set it was checked against. The certification
        // package's `evidence/axiom_audit.json` (P11) re-homes these.
        let audit = harvest
            .iter()
            .find(|(m, _, _)| *m == module)
            .map(|(_, _, a)| a.clone())
            .unwrap_or_default();
        fs::write(
            harvest_out.join(format!("{module}.axiom_audit.json")),
            &audit,
        )
        .map_err(TyuError::Io)?;
        verdict_files.push((module.clone(), path));
        // One parse serves both consumers: the statement accounting and the
        // candidate-evidence ids (§Q10 attribution).
        let (proven, candidates, cand_ids) = certificate_stats(&v2);
        for s in statements.iter_mut().filter(|s| s.module == module) {
            s.unproven = s.rendered.saturating_sub(proven);
            s.proven = proven;
            s.candidates = candidates;
        }
        // P10.2 observability: the per-module candidate evidence
        // (`tyu.candidates/1`) — the candidate-authored certificate ids,
        // re-homed beside the verdicts for the report/package (§Q10
        // attribution; the certification package's `evidence/candidates.json`
        // assembles from these in P11).
        let cand_doc = format!(
            "{{\"schema\":\"tyu.candidates/1\",\"module\":{},\"candidates\":[{}]}}\n",
            json_escape(&module),
            cand_ids
                .iter()
                .map(|i| json_escape(i))
                .collect::<Vec<_>>()
                .join(",")
        );
        fs::write(
            harvest_out.join(format!("{module}.candidates.json")),
            cand_doc,
        )
        .map_err(TyuError::Io)?;
    }

    // Observability (P7.1): the per-module listing — harvested proofs now
    // shrink the `unproven` count.
    for s in &statements {
        if s.proven > 0 {
            eprintln!(
                "tyu: proof statements {}: {} rendered, {} proven, {} omitted, {} unproven",
                s.module, s.rendered, s.proven, s.omitted, s.unproven
            );
        } else {
            eprintln!(
                "tyu: proof statements {}: {} rendered, {} omitted, {} unproven",
                s.module, s.rendered, s.omitted, s.unproven
            );
        }
    }

    Ok((
        ProofStatus::lean("verified", &vendor_digest, statements),
        verdict_files,
    ))
}

/// The certificate accounting of a harvest v2 document, parsed ONCE:
/// `(proof count, candidate-authored count, candidate-authored ids)`.
/// A malformed document reads as all-open (the fail-closed direction —
/// the pipeline re-validates verdicts elsewhere before trusting them).
fn certificate_stats(v2: &str) -> (u32, u32, Vec<String>) {
    use verifier::verdict::{read_verdicts, Authored, Trust};
    match read_verdicts(v2.as_bytes()) {
        Ok(v) => {
            let proof = v.records.iter().filter(|r| r.trust == Trust::Proof).count() as u32;
            let mut candidates = 0u32;
            let mut cand_ids = Vec::new();
            for r in &v.records {
                if r.trust == Trust::Proof && r.authored == Some(Authored::Candidate) {
                    candidates += 1;
                    cand_ids.push(r.id.clone());
                }
            }
            (proof, candidates, cand_ids)
        }
        Err(_) => (0, 0, Vec::new()),
    }
}

/// A minimal JSON string escape (the codec's writers are private).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
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
    out
}

/// The harvest hub: run the generated package's `Harvest.lean` (the
/// `#eval!`d interpreter action) once per module with the environment
/// variables the Lean reader expects, and return the emitted documents:
/// `(module, tyu.verdicts/v2, tyu.axiom-audit/1)`. A nonzero exit is
/// fail-closed — a typed E-code in the harvest error document (E6419 axiom
/// violation, E6420 missing statement) surfaces as [`TyuError::Harvest`];
/// anything else is the E6416 tool-failure class. A tampered proof
/// environment aborts before any check is elided for it.
fn run_harvest(
    package: &LeanPackage,
    module_artifacts: &[PathBuf],
) -> Result<Vec<(String, String, String)>, TyuError> {
    // Harvest input env var names (the Lean reader's contract).
    let gen_var = "TYU_HARVEST_GEN_DIR";
    let obl_var = "TYU_HARVEST_OBL";
    let out_var = "TYU_HARVEST_OUT";
    let cand_var = "TYU_HARVEST_CANDIDATES";
    // P10.2: the fill-candidate attribution set — newline-joined obligation
    // ids from the candidate markers under `proofs/candidates/`. The harvest
    // marks a certificate from that set `authored: "candidate"`.
    let candidates = candidate_ids(&package.project_root).join("\n");
    let mut out = Vec::new();
    for ob in module_artifacts {
        let bytes = fs::read(ob).map_err(TyuError::Io)?;
        let set = verifier::codec::read_obl(&bytes).map_err(|e| {
            TyuError::Build(format!(
                "obligation artifact '{}' invalid (E{}): {e:?}",
                ob.display(),
                e.code()
            ))
        })?;
        let module = set.module.clone();
        let out_path = std::env::temp_dir().join(format!(
            "tyu-harvest-{}-{}.v2.json",
            module,
            std::process::id()
        ));
        let run = Command::new("lake")
            .current_dir(&package.root)
            .arg("env")
            .arg("lean")
            .arg(package.root.join("Harvest.lean"))
            .env(gen_var, &package.gen_dir)
            .env(obl_var, ob)
            .env(out_var, &out_path)
            .env(cand_var, &candidates)
            .output()
            .map_err(|e| {
                TyuError::Build(format!(
                    "E6416: spawning the harvest in '{}': {e}",
                    package.root.display()
                ))
            })?;
        // The `#eval!` harness writes the document at the exit path; a
        // nonzero exit is the fail-closed signal. The harness records the
        // diagnosable reason in the out file (`tyu.harvest-error/1`); a
        // typed registry code in the message (E6419/E6420) surfaces as
        // [`TyuError::Harvest`] — the §6.9 code survives the boundary.
        if !run.status.success() {
            let reason = fs::read_to_string(&out_path)
                .ok()
                .filter(|s| s.contains("tyu.harvest-error/1"));
            let _ = fs::remove_file(&out_path);
            if let Some((code, msg)) = reason.as_deref().and_then(harvest_error_code) {
                return Err(TyuError::Harvest {
                    code,
                    detail: format!("module {module}: {msg}"),
                });
            }
            return Err(TyuError::Build(format!(
                "E6416: harvest failed for module {}{}",
                module,
                match reason {
                    Some(doc) => format!(":\n{}", doc.trim_end()),
                    None => String::new(),
                }
            )));
        }
        let v2 = fs::read(&out_path).map_err(|e| {
            TyuError::Build(format!(
                "E6416: harvest produced no verdicts for {} ({e})",
                module
            ))
        })?;
        let audit_path = audit_out_path(&out_path);
        let audit = fs::read(&audit_path).map_err(|e| {
            TyuError::Build(format!(
                "E6416: harvest produced no axiom-audit evidence for {} ({e})",
                module
            ))
        })?;
        let _ = fs::remove_file(&out_path);
        let _ = fs::remove_file(&audit_path);
        out.push((
            module,
            String::from_utf8(v2)
                .map_err(|_| TyuError::Build("E6416: harvest verdicts not UTF-8".into()))?,
            String::from_utf8(audit)
                .map_err(|_| TyuError::Build("E6416: harvest audit not UTF-8".into()))?,
        ));
    }
    Ok(out)
}

/// The axiom-audit evidence path for a harvest out path: the Lean writer
/// appends `.audit.json` (no new env var — the pair travels together).
fn audit_out_path(out_path: &Path) -> PathBuf {
    let mut name = out_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("harvest")
        .to_string();
    name.push_str(".audit.json");
    out_path.with_file_name(name)
}

/// Extract the typed E-code from a `tyu.harvest-error/1` document: the
/// Lean writer emits `{"schema":…,"message":"<jstr-escaped err>"}`, and the
/// harvest prefixes registry codes (`E6419: …`, `E6420: …`) on its hard
/// failures. Returns `(code, decoded message)` when a leading code is
/// present; `None` (the E6416 tool-failure class) when not — parse
/// failures and metadata mismatches carry no code.
fn harvest_error_code(doc: &str) -> Option<(u32, String)> {
    const KEY: &str = "\"message\":\"";
    let rest = &doc[doc.find(KEY)? + KEY.len()..];
    let mut msg = String::new();
    let mut chars = rest.chars();
    loop {
        match chars.next()? {
            '"' => break,
            '\\' => match chars.next()? {
                'n' => msg.push('\n'),
                't' => msg.push('\t'),
                'r' => msg.push('\r'),
                other => msg.push(other),
            },
            c => msg.push(c),
        }
    }
    let body = msg.trim_start().strip_prefix('E')?;
    let digits: usize = body.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let code = body[..digits].parse::<u32>().ok()?;
    let detail = body[digits..].trim_start_matches(':').trim_start();
    Some((code, detail.to_string()))
}

// ---------------------------------------------------------------------------
// The generated package (P6.1)
// ---------------------------------------------------------------------------

struct LeanPackage {
    root: PathBuf,
    gen_dir: PathBuf,
    project_root: PathBuf,
}

/// Generate (or fast-path) the `.tyu-verify/lean/` package: vendored port
/// library, rendered statements, lakefile, harvest stub, and the generation
/// state. Guarded by the package lock (§7.2). Idempotent and deterministic
/// (FR-16): unchanged files are not rewritten.
fn generate_package(
    project_root: &Path,
    port_dir: &Path,
    artifacts: &[PathBuf],
) -> Result<LeanPackage, TyuError> {
    let root = lean_package_path(project_root);
    let gen_dir = root.join("Gen");

    let _guard = lock_package(&root)?;

    // Fast path: the generation state records the current fingerprint and the
    // package content is present.
    let fingerprint = package_fingerprint(project_root, port_dir, artifacts, &root)?;
    if let Ok(state) = fs::read_to_string(root.join(".tyu-gen.json")) {
        if state.trim() == state_text(&fingerprint).trim() && gen_dir.is_dir() {
            return Ok(LeanPackage {
                root,
                gen_dir,
                project_root: project_root.to_path_buf(),
            });
        }
    }

    fs::create_dir_all(&gen_dir).map_err(TyuError::Io)?;

    // 1. The hash-pinned toolchain file — the port's pin, byte for byte.
    let pin_bytes = fs::read(port_dir.join("lean-toolchain"))
        .map_err(|e| TyuError::Build(format!("reading port lean-toolchain: {e}")))?;
    write_if_changed(&root.join("lean-toolchain"), &pin_bytes)?;

    // 2. The vendored port library (semantics + interfaces).
    vendor_port_lib(port_dir, &root)?;

    // 3. The generated statements + metadata (the port's `gen` renderer).
    render_gen_files(port_dir, artifacts, &root, &gen_dir)?;

    // 4. The lakefile (libs + harvest exe stub; the TyuProofs lib resolves the
    //    developer's own proofs/ directory).
    let lakefile = render_lakefile(project_root)?;
    write_if_changed(&root.join("lakefile.toml"), lakefile.as_bytes())?;

    // 5. P7.1: the harvest entry — the generated package's kernel environment
    //    runs the vendored harvest (`#eval!`d CommandElabM action via
    //    `lake env lean`), and the `Gen.lean` aggregator makes the statements
    //    reachable in it even without developer proofs.
    let gen_modules = gen_modules(gen_dir.as_path())?;
    let has_proofs = project_root.join(PROOFS_DIR).join("proofs.lean").is_file();
    // P10.2: candidate roots (proofs/candidates/*.lean) are importable by
    // the harvest environment so their kernel-checked theorems are found and
    // marked `authored: "candidate"`.
    let candidate_roots: Vec<String> =
        candidate_files(&project_root.join(PROOFS_DIR).join("candidates"))
            .iter()
            .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
            .collect();
    write_if_changed(
        &root.join("Gen.lean"),
        gen_aggregator(&gen_modules).as_bytes(),
    )?;
    write_if_changed(
        &root.join("Harvest.lean"),
        harvest_entry(&gen_modules, has_proofs, &candidate_roots).as_bytes(),
    )?;

    // 6. The generation state — LAST, so a crash mid-regeneration leaves a
    //    mismatched fingerprint and the next build regenerates.
    write_if_changed(
        &root.join(".tyu-gen.json"),
        state_text(&fingerprint).as_bytes(),
    )?;

    Ok(LeanPackage {
        root,
        gen_dir,
        project_root: project_root.to_path_buf(),
    })
}

/// The rendered statement modules currently in `gen_dir` (each
/// `Gen/<Module>.lean`), sorted.
fn gen_modules(gen_dir: &Path) -> Result<Vec<String>, TyuError> {
    let mut out: Vec<String> = Vec::new();
    for entry in fs::read_dir(gen_dir).map_err(TyuError::Io)?.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("lean") {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                out.push(stem.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The `Gen.lean` aggregator: `import Gen.<M>` per rendered module, so the
/// package's lake environment (the harvest's) contains every statement even
/// when no developer proofs exist yet.
fn gen_aggregator(modules: &[String]) -> String {
    let mut s =
        String::from("-- Generated by tyu (PLAN-VERIFY-3 P7.1) — the statement aggregator.\n");
    for m in modules {
        s.push_str("import Gen.");
        s.push_str(m);
        s.push('\n');
    }
    s
}

/// The P7 harvest entry: imports the statements (`Gen`) + the developer's
/// proofs (when present) + the fill candidates (when present) + the vendored
/// harvest, and runs it. The `#eval!`d action reads its inputs from the
/// `TYU_HARVEST_*` environment variables and writes `tyu.verdicts/v2`.
fn harvest_entry(modules: &[String], has_proofs: bool, candidate_roots: &[String]) -> String {
    let mut s = String::new();
    s.push_str("import Tyu.Verdicts.Harvest\n");
    for m in modules {
        s.push_str("import Gen.");
        s.push_str(m);
        s.push('\n');
    }
    if has_proofs {
        s.push_str("import proofs\n");
    }
    for c in candidate_roots {
        s.push_str("import ");
        s.push_str(c);
        s.push('\n');
    }
    s.push('\n');
    s.push_str("-- The harvest (PLAN-VERIFY-3 P7.1): this environment — the\n");
    s.push_str("-- generated statements + the developer's kernel-checked theorems —\n");
    s.push_str("-- is what the harvest audits. Deterministic `tyu.verdicts/v2` out.\n");
    s.push_str("#eval! Tyu.Verdicts.Harvest.run\n");
    s
}

/// The regeneration fingerprint: hashes of (vendored port files, toolchain
/// pin, obligation artifacts, proof files, statement/semantics versions, and
/// the project root — the lakefile embeds the absolute proofs path). A cache
/// key, not an integrity digest (FR-14's SHA-256 rule governs the codec
/// digests, not build invalidation).
fn package_fingerprint(
    project_root: &Path,
    port_dir: &Path,
    artifacts: &[PathBuf],
    root: &Path,
) -> Result<u64, TyuError> {
    let mut items: Vec<(String, u64)> = Vec::new();

    // The vendored surface: every file the package copies from the port.
    for rel in vendored_port_files() {
        let path = port_dir.join(rel);
        items.push((format!("port:{rel}"), hash_file(&path)?));
    }
    items.push((
        "port:lean-toolchain".to_string(),
        hash_file(&port_dir.join("lean-toolchain"))?,
    ));

    // The obligation artifacts (what the statements render from).
    for a in artifacts {
        let name = a.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        items.push((format!("obl:{name}"), hash_file(a)?));
    }

    // The developer's proofs: content + file set.
    items.push((
        "proofs".to_string(),
        proof_files_hash(&project_root.join(PROOFS_DIR)),
    ));

    // Versions and the project root (absolute path in the lakefile).
    items.push((
        "stmt".to_string(),
        cache::fnv1a_u64(verifier::stmt::STMT_SCHEMA.as_bytes()),
    ));
    items.push((
        "semantics".to_string(),
        cache::fnv1a_u64(verifier::semantics::SEMANTICS_VERSION.as_bytes()),
    ));
    let root_str = root.to_string_lossy().into_owned();
    items.push(("root".to_string(), cache::fnv1a_u64(root_str.as_bytes())));

    items.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (k, v) in &items {
        h ^= v;
        h = h.wrapping_mul(0x100000001b3);
        h ^= cache::fnv1a_u64(k.as_bytes());
        h = h.wrapping_mul(0x100000001b3);
    }
    Ok(h)
}

/// The port files the generated package vendors (deterministic list; the
/// golden corpus is port-exe/fixture material and stays out). P7.1 adds the
/// renderer itself + the harvest library (the generated statements import
/// `Tyu.Gen.Render`'s canonical-encoding helpers transitively through
/// `Tyu.Gen.Stmt`, and the harvest reuses the SAME canonical surface — one
/// source of truth across encoder/renderer/harvest).
const fn vendored_port_files() -> &'static [&'static str] {
    &[
        "Tyu.lean",
        "Tyu/IR/Op.lean",
        "Tyu/IR/Semantics.lean",
        "Tyu/IR/Target.lean",
        "Tyu/Mem.lean",
        "Tyu/Src.lean",
        "Tyu/Step.lean",
        "Tyu/Sound.lean",
        "Tyu/Stackmeta.lean",
        "Tyu/Conformance/Cfg.lean",
        "Tyu/Conformance/Interval.lean",
        "Tyu/Conformance/IntervalLaws.lean",
        "Tyu/Conformance/Json.lean",
        "Tyu/Conformance/Parse.lean",
        "Tyu/Conformance/Runner.lean",
        "Tyu/Conformance/Step.lean",
        "Tyu/Conformance/VectorRun.lean",
        "Tyu/Gen/Sha256.lean",
        "Tyu/Gen/Stmt.lean",
        "Tyu/Gen/Render.lean",
        "Tyu/Verdicts/Harvest.lean",
    ]
}

/// Copy the vendored port library files into the package (temp + rename;
/// unchanged bytes are not rewritten — mtime-stability keeps lake
/// incremental).
fn vendor_port_lib(port_dir: &Path, root: &Path) -> Result<(), TyuError> {
    for rel in vendored_port_files() {
        let src = port_dir.join(rel);
        let bytes = fs::read(&src).map_err(|e| {
            TyuError::Build(format!(
                "E6416: port library file '{}' unreadable: {e}",
                src.display()
            ))
        })?;
        let dst = root.join(rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(TyuError::Io)?;
        }
        write_if_changed(&dst, &bytes)?;
    }
    Ok(())
}

/// Run the port's `gen` renderer over the artifacts into `gen_dir`,
/// then prune stale Gen files (modules no longer in the artifact set).
fn render_gen_files(
    port_dir: &Path,
    artifacts: &[PathBuf],
    root: &Path,
    gen_dir: &Path,
) -> Result<(), TyuError> {
    let gen_bin = ensure_gen_exe(port_dir)?;

    // Clean the Gen dir so a removed module cannot leave a stale statement
    // behind (determinism: the dir is a pure function of the artifact set).
    if gen_dir.exists() {
        for entry in fs::read_dir(gen_dir).map_err(TyuError::Io)?.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("lean")
                || p.extension().and_then(|e| e.to_str()) == Some("json")
            {
                let _ = fs::remove_file(&p);
            }
        }
    }
    fs::create_dir_all(gen_dir).map_err(TyuError::Io)?;

    let mut cmd = Command::new(&gen_bin);
    cmd.arg("--render").arg("--obl");
    for a in artifacts {
        cmd.arg(a);
    }
    cmd.arg("--out").arg(gen_dir);
    let out = cmd.output().map_err(|e| {
        TyuError::Build(format!(
            "E6416: running gen renderer '{}': {e}",
            gen_bin.display()
        ))
    })?;
    if !out.status.success() {
        let tail = String::from_utf8_lossy(&out.stdout)
            .lines()
            .rev()
            .take(16)
            .collect::<Vec<_>>()
            .join("\n");
        return Err(TyuError::Build(format!(
            "E6416: gen renderer failed (module statements not rendered):\n{tail}"
        )));
    }

    // The module set the artifacts describe (the renderer writes
    // `<Module>.lean` by the artifact's `module` field).
    let mut expected: BTreeSet<String> = BTreeSet::new();
    for a in artifacts {
        let bytes = fs::read(a).map_err(TyuError::Io)?;
        match verifier::codec::read_obl(&bytes) {
            Ok(set) => {
                expected.insert(set.module);
            }
            Err(e) => {
                return Err(TyuError::Build(format!(
                    "obligation artifact '{}' invalid (E{}): {e:?}",
                    a.display(),
                    e.code()
                )));
            }
        }
    }
    for entry in fs::read_dir(gen_dir).map_err(TyuError::Io)?.flatten() {
        let p = entry.path();
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };
        if p.extension().and_then(|e| e.to_str()) == Some("lean") && !expected.contains(&stem) {
            let _ = fs::remove_file(&p);
        }
    }

    let _ = root; // (gen output lands in gen_dir; root kept for API symmetry)
    Ok(())
}

/// Locate (building if necessary) the port's `gen` renderer executable.
fn ensure_gen_exe(port_dir: &Path) -> Result<PathBuf, TyuError> {
    let bin = port_dir.join(".lake").join("build").join("bin").join("gen");
    if bin.is_file() {
        return Ok(bin);
    }
    eprintln!(
        "tyu: building the port's gen renderer in '{}'",
        port_dir.display()
    );
    let status = Command::new("lake")
        .current_dir(port_dir)
        .args(["build", "gen"])
        .status()
        .map_err(|e| {
            TyuError::Build(format!(
                "E6416: spawning `lake build gen` in '{}': {e}",
                port_dir.display()
            ))
        })?;
    if !status.success() {
        return Err(TyuError::Build(format!(
            "E6416: `lake build gen` failed in '{}'",
            port_dir.display()
        )));
    }
    if !bin.is_file() {
        return Err(TyuError::Build(format!(
            "E6416: gen renderer not produced in '{}'",
            port_dir.display()
        )));
    }
    Ok(bin)
}

/// The generated lakefile: the vendored `Tyu` lib, the generated `Gen` lib,
/// the developer's `TyuProofs` lib (roots enumerated from `proofs/*.lean`,
/// sorted — deterministic), and the `harvest` exe stub.
fn render_lakefile(project_root: &Path) -> Result<String, TyuError> {
    let proofs_dir = project_root.join(PROOFS_DIR);
    let mut text = String::new();
    text.push_str("# Generated by tyu (PLAN-VERIFY-3 P6.1) — DO NOT EDIT.\n");
    text.push_str("# The package is regenerated every `tyu build --verify-tool=lean`.\n");
    text.push_str("# libs: Tyu (vendored port) + Gen (generated statements) + TyuProofs\n");
    text.push_str("#       (developer-owned proofs/, enumerated roots) + the harvest stub.\n");
    text.push('\n');
    text.push_str("name = \"tyu-verify\"\n");
    text.push_str("version = \"0.1.0\"\n");
    text.push('\n');
    text.push_str("[[lean_lib]]\n");
    text.push_str("name = \"Tyu\"\n");
    text.push_str("srcDir = \".\"\n");
    text.push_str("roots = [\"Tyu\"]\n");
    text.push('\n');
    text.push_str("[[lean_lib]]\n");
    text.push_str("name = \"Gen\"\n");
    text.push_str("srcDir = \".\"\n");
    text.push_str("roots = [\"Gen\"]\n");

    let mut roots: Vec<String> = Vec::new();
    if proofs_dir.is_dir() {
        for entry in fs::read_dir(&proofs_dir).map_err(TyuError::Io)?.flatten() {
            let p = entry.path();
            if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("lean") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    roots.push(stem.to_string());
                }
            }
        }
    }
    // P10.2: the fill-generated candidates (`proofs/candidates/<id>.lean`)
    // are package roots so `lake build` kernel-checks them and the harvest
    // finds their theorems (marking `authored: "candidate"`). They are never
    // merged into the developer-owned `proofs.lean` root.
    let candidates_dir = proofs_dir.join("candidates");
    if candidates_dir.is_dir() {
        for entry in fs::read_dir(&candidates_dir)
            .map_err(TyuError::Io)?
            .flatten()
        {
            let p = entry.path();
            if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("lean") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    roots.push(stem.to_string());
                }
            }
        }
    }
    roots.sort();
    roots.dedup();
    if !roots.is_empty() {
        // The absolute proofs path: lake resolves srcDir exactly once, from
        // wherever the build runs.
        let abs = if proofs_dir.is_absolute() {
            proofs_dir.clone()
        } else {
            std::env::current_dir()
                .map_err(TyuError::Io)?
                .join(&proofs_dir)
        };
        text.push_str("\n[[lean_lib]]\n");
        text.push_str("name = \"TyuProofs\"\n");
        text.push_str("srcDir = \"");
        text.push_str(&toml_escape(&abs.to_string_lossy()));
        text.push_str("\"\n");
        text.push_str("roots = [");
        for (i, r) in roots.iter().enumerate() {
            if i != 0 {
                text.push_str(", ");
            }
            text.push('"');
            text.push_str(&toml_escape(r));
            text.push('"');
        }
        text.push_str("]\n");
    }
    Ok(text)
}

/// Escape a TOML basic string minimally (control chars and `"`/`\`).
fn toml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Package lock (§7.2)
// ---------------------------------------------------------------------------

/// A create-exclusive package lock; dropped (released) on scope exit.
#[derive(Debug)]
struct LockGuard {
    path: PathBuf,
}

/// Acquire the package lock: create-exclusive on `.tyu-verify/lean/`, waiting
/// up to the timeout (env-overridable for tests) then E6416. A lock whose
/// recorded PID is dead is stolen (a crashed build cannot wedge the package).
fn lock_package(root: &Path) -> Result<LockGuard, TyuError> {
    fs::create_dir_all(root).map_err(TyuError::Io)?;
    let lock_path = root.join(".tyu-gen.lock");
    let timeout = std::env::var(LOCK_TIMEOUT_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_LOCK_TIMEOUT);
    let deadline = Instant::now() + timeout;

    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                use std::io::Write as _;
                let _ = f.write_all(format!("{}\n", std::process::id()).as_bytes());
                return Ok(LockGuard {
                    path: lock_path.clone(),
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if lock_dead(&lock_path) {
                    let _ = fs::remove_file(&lock_path);
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err(TyuError::Build(format!(
                        "E6416: generated proof package '{}' is locked by another \
                         build ({} ms elapsed): concurrent tyu builds share the \
                         package lock, §7.2",
                        root.display(),
                        timeout.as_millis()
                    )));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(TyuError::Build(format!(
                    "E6416: cannot create package lock '{}': {e}",
                    lock_path.display()
                )));
            }
        }
    }
}

/// True when the lock's recorded owner PID is gone (a crashed build).
fn lock_dead(lock_path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(lock_path) else {
        return true;
    };
    let Ok(pid) = text.trim().parse::<i32>() else {
        return true;
    };
    // `kill(0)` probes existence without signalling.
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    !alive
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

// ---------------------------------------------------------------------------
// Vendored-library verification (§Q5 digest-recorded / T-F2 fail-closed)
// ---------------------------------------------------------------------------

/// Verify every vendored package file is byte-identical to the port's and
/// return the SHA-256 (hex) over the vendored surface as committed — the
/// digest the build report records (`proof.vendor_digest`). A tampered
/// `Tyu/*.lean` in the package is E6418 fail-closed (silent wrong-semantics
/// proving is the exact failure this gate exists to prevent).
fn verify_vendored_files(package: &LeanPackage, port_dir: &Path) -> Result<String, TyuError> {
    let mut digests: Vec<(String, [u8; 32])> = Vec::new();
    for rel in vendored_port_files() {
        let port_bytes = fs::read(port_dir.join(rel)).map_err(|e| {
            TyuError::Build(format!(
                "E6416: port library file '{}' unreadable: {e}",
                port_dir.join(rel).display()
            ))
        })?;
        let pkg_bytes = fs::read(package.root.join(rel)).map_err(|e| {
            TyuError::Build(format!(
                "E6418: vendored file '{}' missing from the package: {e}",
                package.root.join(rel).display()
            ))
        })?;
        if pkg_bytes != port_bytes {
            return Err(TyuError::Build(format!(
                "E6418: vendored semantics file '{}' disagrees with the port \
                 (tampered Tyu library?) — refusing to elaborate proofs against \
                 the wrong semantics",
                rel
            )));
        }
        digests.push((rel.to_string(), verifier::stmt::sha256(&pkg_bytes)));
    }
    digests.sort();
    let mut stream = Vec::with_capacity(digests.len() * 40);
    for (rel, digest) in &digests {
        stream.extend_from_slice(rel.as_bytes());
        stream.extend_from_slice(digest);
    }
    Ok(verifier::stmt::hex32(verifier::stmt::sha256(&stream)))
}

// ---------------------------------------------------------------------------
// Gen-digest verification (E6418) + statement accounting
// ---------------------------------------------------------------------------

/// Recompute every obligation's canonical `statement_hash` (P1.1 encoder) and
/// verify the rendered Gen surface carries it — the pre-lake tamper gate.
/// Returns the per-module statement accounting. Any mismatch is `E6418`
/// (fail-closed: a mutated `Gen` file or a renderer/encoder drift aborts
/// before lake runs).
fn verify_gen_digests(
    package: &LeanPackage,
    artifacts: &[PathBuf],
) -> Result<Vec<ModuleStatements>, TyuError> {
    let mut out: Vec<ModuleStatements> = Vec::new();
    for a in artifacts {
        let bytes = fs::read(a).map_err(TyuError::Io)?;
        let set = verifier::codec::read_obl(&bytes).map_err(|e| {
            TyuError::Build(format!(
                "obligation artifact '{}' invalid (E{}): {e:?}",
                a.display(),
                e.code()
            ))
        })?;
        let module = &set.module;

        let lean_path = package.gen_dir.join(format!("{module}.lean"));
        let meta_path = package.gen_dir.join(format!("{module}.gen.json"));
        if !lean_path.is_file() || !meta_path.is_file() {
            return Err(TyuError::Build(format!(
                "E6418: generated statement surface missing for module {} \
                 (expected '{}' and '{}')",
                module,
                lean_path.display(),
                meta_path.display()
            )));
        }

        let lean_text = fs::read_to_string(&lean_path).map_err(TyuError::Io)?;
        let meta = parse_gen_meta(&fs::read(&meta_path).map_err(TyuError::Io)?)?;
        let stored_hashes = extract_statement_hashes(&lean_text);

        let mut rendered: u32 = 0;
        let mut omitted: u32 = 0;
        for o in &set.obligations {
            let word_ir_hash = set
                .facts
                .words
                .iter()
                .find(|w| w.name == o.site.word)
                .map(|w| sha256_hex16(w.ir.as_bytes()))
                .unwrap_or_default();
            // §Q3 relativity: the statement is bound to the artifact's own
            // (target, model_semantics) identity — the consuming build's
            // mismatch is the E6421 path at langc, a different surface.
            let ctx = StatementContext::for_obligation(
                module,
                &set.target,
                &set.model_semantics,
                &word_ir_hash,
                o,
            );
            let expected_hash = ctx.statement_hash_hex(&o.formula);

            let row = meta
                .statements
                .iter()
                .find(|s| s.id == o.id)
                .ok_or_else(|| {
                    TyuError::Build(format!(
                        "E6418: module {} — obligation {} missing from Gen metadata",
                        module, o.id
                    ))
                })?;
            if row.omitted {
                omitted = omitted.saturating_add(1);
                continue;
            }
            rendered = rendered.saturating_add(1);
            if row.statement_hash != expected_hash {
                return Err(TyuError::Build(format!(
                    "E6418: statement hash mismatch for {} — Gen metadata \
                     records '{}', the canonical encoder computes '{}'",
                    o.id, row.statement_hash, expected_hash
                )));
            }
            if !stored_hashes.contains(&expected_hash) {
                return Err(TyuError::Build(format!(
                    "E6418: statement hash for {} not carried in '{}' (tampered Gen?)",
                    o.id,
                    lean_path.display()
                )));
            }
        }
        out.push(ModuleStatements {
            module: module.clone(),
            rendered,
            omitted,
            proven: 0, // filled by the P7 harvest (certificate counts).
            unproven: rendered,
            candidates: 0, // filled by the P7 harvest (P10.2 attribution).
        });
    }
    out.sort_by(|a, b| a.module.cmp(&b.module));
    Ok(out)
}

/// The `statements` array of a `<Module>.gen.json` metadata file.
/// A string-keyed scan over a JSON-object substring: the value of `"key"`
/// (string form, unescaped conservatively). `None` = the key is absent.
/// The gen metadata is toolchain-produced (the port's `gen` renderer), so
/// the reader only needs the producer's own shape — hand-rolled per the
/// FR-15 codec discipline, mirroring the port's
/// `Tyu.Automation.Corpus.parseGenMetaRows`.
fn json_string_field(obj: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut from = 0usize;
    while let Some(k) = obj[from..].find(&needle) {
        let after = &obj[from + k + needle.len()..];
        let after = after.trim_start();
        let after = after.strip_prefix(':')?.trim_start();
        if !after.starts_with('"') {
            // Not a string value (a different row's similar key) — skip.
            from += k + needle.len();
            continue;
        }
        let mut out = String::new();
        let mut chars = after[1..].chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => return Some(out),
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some(other) => out.push(other),
                    None => return None,
                },
                c => out.push(c),
            }
        }
        return None; // Unterminated string (malformed) — treat as absent.
    }
    None
}

/// The boolean value of `"key"` in a JSON-object substring.
fn json_bool_field(obj: &str, key: &str) -> Option<bool> {
    let needle = format!("\"{key}\"");
    let k = obj.find(&needle)?;
    let after = obj[k + needle.len()..].trim_start();
    let after = after.strip_prefix(':')?.trim_start();
    if after.starts_with("true") {
        Some(true)
    } else if after.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// The `{ ... }` spans of a JSON array substring (string-aware brace
/// matching; the producer never nests braces inside strings). A `}` with
/// no open object (the document's root close, trailing the array) is
/// ignored.
fn json_object_spans(arr: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, c) in arr.char_indices() {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    out.push(&arr[start..=i]);
                }
            }
            _ => {}
        }
    }
    out
}

/// The `statements` array of a `<Module>.gen.json` metadata document
/// (`tyu.gen/1`, written by the port's `gen` renderer).
fn parse_gen_meta(bytes: &[u8]) -> Result<GenMeta, TyuError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| TyuError::Build("E6418: unparseable Gen metadata: not UTF-8".into()))?;
    let schema = json_string_field(text, "schema").unwrap_or_default();
    if schema != "tyu.gen/1" {
        return Err(TyuError::Build(format!(
            "E6418: unknown Gen metadata schema '{schema}' (expected tyu.gen/1)"
        )));
    }
    let mut statements = Vec::new();
    if let Some(arr_start) = text.find("\"statements\"") {
        let rest = text[arr_start..].trim_start();
        if let Some(bracket) = rest.find('[') {
            for row in json_object_spans(&rest[bracket..]) {
                let id = json_string_field(row, "id").unwrap_or_default();
                let omitted = json_bool_field(row, "omitted").unwrap_or(false);
                let statement_hash = json_string_field(row, "statement_hash").unwrap_or_default();
                statements.push(GenStatementRow {
                    id,
                    omitted,
                    statement_hash,
                });
            }
        }
    }
    Ok(GenMeta { statements })
}

#[derive(Debug)]
struct GenMeta {
    statements: Vec<GenStatementRow>,
}

#[derive(Debug)]
struct GenStatementRow {
    id: String,
    omitted: bool,
    statement_hash: String,
}

/// Extract the `statement_hash: <hex>` annotations the renderer writes into
/// each generated statement's doc comment.
fn extract_statement_hashes(lean_text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = lean_text;
    while let Some(idx) = rest.find("statement_hash: ") {
        rest = &rest[idx + "statement_hash: ".len()..];
        let hex_len = rest
            .bytes()
            .take_while(|b| b.is_ascii_hexdigit())
            .count()
            .min(64);
        if hex_len == 64 {
            out.insert(rest[..64].to_string());
        }
        if hex_len > 0 {
            rest = &rest[hex_len..];
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Build state + helper I/O
// ---------------------------------------------------------------------------

/// The `.tyu-gen.json` generation-state text for a fingerprint.
fn state_text(fingerprint: &u64) -> String {
    format!(
        "{{\"schema\":\"{PKG_STATE_SCHEMA}\",\"fingerprint\":\"{fingerprint:016x}\",\
        \"stmt\":\"{}\",\"semantics\":\"{}\"}}\n",
        verifier::stmt::STMT_SCHEMA,
        verifier::semantics::SEMANTICS_VERSION
    )
}

/// Write `bytes` to `path` only when the existing content differs (temp +
/// rename; mtime-stability keeps lake incremental — FR-16).
fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), TyuError> {
    if let Ok(existing) = fs::read(path) {
        if existing == bytes {
            return Ok(());
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(TyuError::Io)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes).map_err(TyuError::Io)?;
    fs::rename(&tmp, path).map_err(|e| {
        TyuError::Build(format!(
            "re-homing '{}' -> '{}': {}",
            tmp.display(),
            path.display(),
            e
        ))
    })?;
    Ok(())
}

fn hash_file(path: &Path) -> Result<u64, TyuError> {
    cache::content_hash(path)
}

/// The deterministic proof-files hash (§7.2 `proof_files_hash`): FNV-1a over
/// the sorted `(relative path, content hash)` pairs of everything under
/// `proofs/`. A cache key for the TyuProofs lib / verdicts cache — not an
/// integrity digest.
pub fn proof_files_hash(proofs_dir: &Path) -> u64 {
    let mut items: Vec<(String, u64)> = Vec::new();
    if let Ok(rd) = fs::read_dir(proofs_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                continue;
            }
            let rel = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?")
                .to_string();
            items.push((rel, cache::content_hash(&p).unwrap_or(0)));
        }
    }
    items.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (rel, ch) in items {
        h ^= ch;
        h = h.wrapping_mul(0x100000001b3);
        h ^= cache::fnv1a_u64(rel.as_bytes());
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// ---------------------------------------------------------------------------
// Toolchain resolution (§7.2; E6416)
// ---------------------------------------------------------------------------

/// Verify the Lean toolchain is present and matches the port's pin (the
/// `lean-toolchain` reference) — `elan which lean` when elan manages the
/// toolchain, else the PATH `lean`/`lake` with a version check. Mismatch or
/// absence is E6416 with the pin in the diagnostic.
pub fn ensure_lean_toolchain(pin: &str) -> Result<(), TyuError> {
    let want = pin
        .rsplit(':')
        .next()
        .unwrap_or(pin)
        .trim()
        .trim_start_matches('v')
        .to_string();
    // Elan-managed: `elan which lean` resolves the pinned toolchain from the
    // package's lean-toolchain file when one is present in the CWD.
    let lean_bin = if let Some(elan) = crate::toolchain::find_in_path("elan") {
        if let Ok(out) = Command::new(&elan).arg("which").arg("lean").output() {
            if out.status.success() {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path.is_empty() {
                    Some(PathBuf::from(path))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    let lean_bin = match lean_bin {
        Some(b) => Some(b),
        None => crate::toolchain::find_in_path("lean"),
    };
    check_lean_toolchain(pin, &want, lean_bin.as_deref(), lake_on_path())
}

/// `check_lean_toolchain` is the decision core (split from discovery so the
/// missing-toolchain / version-mismatch / lake-missing E6416 paths are unit-
/// testable without a real Lean installation): `lean_bin` is `None` when no
/// `lean` resolved (elan or PATH); `lake_present` reports `lake` on PATH.
fn check_lean_toolchain(
    pin: &str,
    want: &str,
    lean_bin: Option<&Path>,
    lake_present: bool,
) -> Result<(), TyuError> {
    let lean_bin = lean_bin.ok_or_else(|| {
        TyuError::Build(format!(
            "E6416: Lean toolchain unavailable — pin '{}' requires `lean` \
             (via elan or PATH); the proof pipeline cannot run",
            pin
        ))
    })?;
    let got = lean_version(lean_bin)
        .ok_or_else(|| TyuError::Build(format!("E6416: cannot probe '{}'", lean_bin.display())))?;
    if !same_version(&got, want) {
        return Err(TyuError::Build(format!(
            "E6416: Lean toolchain mismatch — pin '{}' ({}), resolved '{}' is {} \
             — run `elan toolchain install {}` (or align the pinned toolchain)",
            pin,
            want,
            lean_bin.display(),
            got,
            pin
        )));
    }
    if !lake_present {
        return Err(TyuError::Build(format!(
            "E6416: `lake` not found in PATH — required to build the generated \
             proof package (pin {})",
            pin
        )));
    }
    Ok(())
}

/// `lake` present on PATH (the elaborating build step).
fn lake_on_path() -> bool {
    crate::toolchain::find_in_path("lake").is_some()
}

/// `lean --version` → `"4.27.0"`-style version string.
fn lean_version(path: &Path) -> Option<String> {
    let out = Command::new(path).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let v = text
        .strip_prefix("Lean (version ")
        .and_then(|r| r.split(',').next())?
        .trim();
    Some(v.to_string())
}

/// Prefix-equal version comparison on dot-separated numeric segments: the
/// pin's segments must be a prefix of the installed toolchain's (`want`
/// `"4.27"` is satisfied by `got` `"4.27.0"`; a full pin compares exactly).
fn same_version(got: &str, want: &str) -> bool {
    let seg = |s: &str| -> Vec<u32> {
        s.trim()
            .split('.')
            .filter_map(|p| {
                p.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .ok()
            })
            .collect()
    };
    let g = seg(got);
    let w = seg(want);
    !w.is_empty() && g.len() >= w.len() && g.iter().zip(w.iter()).all(|(a, b)| a == b)
}

/// The elaborating lake build of the generated package: the `Gen` aggregator
/// (every generated statement) and, when the developer has proofs, the
/// `proofs` root. Any failure — a developer proof that does not elaborate, a
/// vendored-file breakage, a lake crash — is E6416 with the output captured
/// (fail-closed: the build stops before any check is elided for an unproven
/// statement).
fn run_lake_build(package: &LeanPackage) -> Result<(), TyuError> {
    let mut targets = vec![
        "build".to_string(),
        "Gen".to_string(),
        "Tyu.Verdicts.Harvest".to_string(),
    ];
    // `proofs` is a TyuProofs root module — a lake target only when present.
    if package
        .project_root
        .join(PROOFS_DIR)
        .join("proofs.lean")
        .is_file()
    {
        targets.push("proofs".to_string());
    }
    // P10.2: the fill candidates are TyuProofs roots too — build them so the
    // kernel checks their theorems before the harvest (fail-closed on an
    // un-elaborating candidate exactly like any developer proof).
    for c in candidate_files(&package.project_root.join(PROOFS_DIR).join("candidates")) {
        if let Some(stem) = c.file_stem().and_then(|s| s.to_str()) {
            targets.push(stem.to_string());
        }
    }
    let out = Command::new("lake")
        .current_dir(&package.root)
        .args(&targets)
        .output()
        .map_err(|e| {
            TyuError::Build(format!(
                "E6416: spawning `lake build` in '{}': {e}",
                package.root.display()
            ))
        })?;
    if !out.status.success() {
        let mut tail = String::new();
        for stream in [&out.stdout, &out.stderr] {
            let text = String::from_utf8_lossy(stream);
            tail.push_str(&text);
        }
        let tail = tail
            .lines()
            .rev()
            .take(32)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(TyuError::Build(format!(
            "E6416: `lake build` failed in '{}' — the generated \
             statements and/or the developer proofs did not elaborate:\n{}",
            package.root.display(),
            tail
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Harvest boundary (P6.2: wired, honest, tested — P7 replaces the error)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// ℹ The P6 harvest *boundary* (E6416 harvest-not-built) was superseded in P7:
// the real harvest (`run_harvest`) consumes the kernel environment now.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tyu-proof-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn proof_files_hash_is_deterministic_and_sensitive() {
        let dir = temp_dir("pfh");
        let proofs = dir.join("proofs");
        fs::create_dir_all(&proofs).unwrap();
        fs::write(proofs.join("Bank.lean"), "theorem a : True := by trivial\n").unwrap();
        fs::write(proofs.join("proofs.lean"), "import Bank\n").unwrap();
        let h1 = proof_files_hash(&proofs);
        let h2 = proof_files_hash(&proofs);
        assert_eq!(h1, h2, "same tree → same hash");
        fs::write(proofs.join("Bank.lean"), "-- changed\n").unwrap();
        let h3 = proof_files_hash(&proofs);
        assert_ne!(h1, h3, "content change must change the hash");
        fs::write(proofs.join("Loop.lean"), "theorem b : True := by trivial\n").unwrap();
        let h4 = proof_files_hash(&proofs);
        assert_ne!(h1, h4, "file-set change must change the hash");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn candidate_markers_are_scanned_and_attributed() {
        // `candidate_ids` parses the `-- tyu:candidate obligation=<id>`
        // markers (sorted, deduped); `certificate_stats` reads the
        // attribution from a harvest v2 document.
        // read the attribution from a harvest v2 document.
        let dir = temp_dir("cand");
        fs::create_dir_all(dir.join("proofs").join("candidates")).unwrap();
        fs::write(
            dir.join("proofs/candidates/P1.lean"),
            "-- tyu:candidate obligation=A::w::subtype-range::0\nimport Gen.A\n",
        )
        .unwrap();
        fs::write(
            dir.join("proofs/candidates/P2.lean"),
            "-- tyu:candidate obligation=B::w::contract-post::1\nimport Gen.B\n",
        )
        .unwrap();
        // A marker-less file (a promoted/completed candidate) is ignored.
        fs::write(
            dir.join("proofs/candidates/P3.lean"),
            "theorem x : True := by trivial\n",
        )
        .unwrap();
        let ids = candidate_ids(&dir);
        assert_eq!(
            ids,
            vec![
                "A::w::subtype-range::0".to_string(),
                "B::w::contract-post::1".to_string()
            ],
            "markers scanned, sorted, deduped"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn certificate_counts_split_developer_and_candidate() {
        // A crafted `tyu.verdicts/v2` document with one developer and one
        // candidate certificate.
        let v2 = r#"{"schema":"tyu.verdicts/v2","certifier":{"class":"port","name":"lean","recognition":"tyu-port/lean/1","tool":{"name":"harvest","version":"0.1.0"},"toolchain":"t"},"semantics":"tyu.ir-sem/1.0","stmt":"tyu.stmt/1.0","target":"x86_64-unknown-none","model_semantics":"unmodeled","verdicts":[
          {"id":"A::w::subtype-range::0","id_hash":"h1","status":"discharged","trust":"proof","method":"certificate","surface":"ir","statement_hash":"hh","authored":"developer","proof":{"kind":"certificate","statement":"tyu.stmt/1.0"}},
          {"id":"B::w::contract-post::1","id_hash":"h2","status":"discharged","trust":"proof","method":"certificate","surface":"ir","statement_hash":"hh","authored":"candidate","proof":{"kind":"certificate","statement":"tyu.stmt/1.0"}}
        ]}"#;
        let (proven, candidates, cand_ids) = certificate_stats(v2);
        assert_eq!(proven, 2);
        assert_eq!(candidates, 1);
        assert_eq!(cand_ids, vec!["B::w::contract-post::1".to_string()]);
    }

    #[test]
    fn gen_meta_reader_is_hand_rolled_and_strict() {
        // FR-15: the `tyu.gen/1` reader is hand-rolled (no JSON crate) and
        // keeps the producer's contract: schema gate, per-row fields,
        // `omitted` defaulting false, unknown keys skipped.
        let doc = br#"{
  "schema": "tyu.gen/1",
  "module": "Bank",
  "statements": [
  { "id": "Bank::clamp::subtype-range::0", "id_hash": "cd26", "omitted": true, "reason": "opaque-site", "statement_hash": "" },
  { "id": "Bank::clamp::subtype-range::1", "id_hash": "cd27", "def": "stmt_Bank_clamp_subtype_range_1", "omitted": false, "statement_hash": "1fe4" },
  { "id": "Bank::bounded_inc::subtype-range::0", "def": "stmt_Bank_bounded_inc_subtype_range_0", "statement_hash": "beef" }
  ]}"#;
        let meta = parse_gen_meta(doc).expect("parse");
        assert_eq!(meta.statements.len(), 3);
        assert_eq!(meta.statements[0].id, "Bank::clamp::subtype-range::0");
        assert!(meta.statements[0].omitted);
        assert!(!meta.statements[1].omitted);
        assert_eq!(meta.statements[1].statement_hash, "1fe4");
        // `omitted` absent ⇒ false (the default).
        assert!(!meta.statements[2].omitted);
        // A wrong schema is the E6418 class.
        let err = parse_gen_meta(b"{\"schema\": \"tyu.gen/9\", \"statements\": []}").unwrap_err();
        assert!(err.to_string().contains("E6418"), "err: {err}");
    }
    // A hermetic root: either the port is absent (repo without the
    // verification tree) or the pass-1 artifacts are missing — both are
    // the honest E6416 fail-closed class.
    #[test]
    fn proof_fill_requires_a_port_and_artifacts() {
        let dir = temp_dir("fill");
        fs::create_dir_all(dir.join("proofs")).unwrap();
        let err = proof_fill(
            &dir,
            None,
            &[],
            None,
            None,
            codegen_core::Target::X86_64UnknownNone,
        )
        .unwrap_err();
        assert!(err.to_string().contains("E6416"), "err: {err}");
        let msg = err.to_string();
        assert!(
            msg.contains("Lean verification port missing")
                || msg.contains("no obligation artifacts"),
            "must name the missing precondition: {msg}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn harvest_error_code_surfaces_the_typed_registry_code() {
        // The Lean writer's exact shape: a jstr-escaped message in the
        // `tyu.harvest-error/1` document, prefixed with the registry code
        // on hard failures.
        let (code, msg) = harvest_error_code(
            "{\"schema\":\"tyu.harvest-error/1\",\"message\":\"E6419: axiom audit \
             failed for obl_Tiny_x — axioms: [\\\"sorryAx\\\"]\"}",
        )
        .expect("typed code present");
        assert_eq!(code, 6419);
        assert_eq!(
            msg, "axiom audit failed for obl_Tiny_x — axioms: [\"sorryAx\"]",
            "escapes decode"
        );

        let (code, msg) = harvest_error_code(
            "{\"schema\":\"tyu.harvest-error/1\",\"message\":\"E6420: statement def \
             missing from environment: Tyu.Gen.Corpus.Tiny.stmt_x\"}",
        )
        .expect("typed code present");
        assert_eq!(code, 6420);
        assert!(msg.starts_with("statement def missing"), "msg: {msg}");

        // No leading code ⇒ the E6416 tool-failure class (parse failures,
        // metadata mismatches).
        assert!(harvest_error_code(
            "{\"schema\":\"tyu.harvest-error/1\",\"message\":\"artifact parse: bad\"}"
        )
        .is_none());
        assert!(harvest_error_code("not a harvest document").is_none());
    }

    #[test]
    fn version_check_matches_and_rejects() {
        assert!(same_version("4.27.0", "4.27.0"));
        assert!(same_version("4.27.0", "4.27"), "pin prefix satisfies");
        assert!(!same_version("4.26.0", "4.27.0"));
        assert!(!same_version("4.27.1", "4.27.0"));
        assert!(
            !same_version("4.27.0", "4.27.0.1"),
            "installed short of pin"
        );
    }

    #[test]
    fn write_if_changed_skips_identical_bytes_and_preserves_mtime() {
        let dir = temp_dir("wic");
        let p = dir.join("f.txt");
        write_if_changed(&p, b"hello").unwrap();
        let m1 = fs::metadata(&p).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(40));
        write_if_changed(&p, b"hello").unwrap();
        let m2 = fs::metadata(&p).unwrap().modified().unwrap();
        assert_eq!(m1, m2, "unchanged bytes must not be rewritten");
        write_if_changed(&p, b"world").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"world");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_statement_hashes_scans_doc_comments() {
        let text = "-- /-- statement: X\n    statement_hash: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa -/\n";
        let hashes = extract_statement_hashes(text);
        assert!(hashes.contains("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert!(!extract_statement_hashes("no hashes here").contains("a"));
    }

    #[test]
    fn toml_escape_quotes_and_backslashes() {
        assert_eq!(toml_escape("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(toml_escape("plain/path"), "plain/path");
    }

    // -------------------------------------------------------------------
    // Hermetic pipeline tests (no lake/lean needed — the port's committed
    // corpus goldens + a synthetic port fixture stand in).
    // -------------------------------------------------------------------

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    fn port_dir() -> PathBuf {
        workspace_root()
            .join("verification")
            .join("ports")
            .join("lean")
    }

    /// The corpus golden material the E6418 check runs against: the real
    /// renderer output (gen metadata + Gen text) and the artifact it was
    /// rendered from — pinned by the P5 drift tests, so a mismatch here is a
    /// genuine encoder↔renderer disagreement.
    fn corpus_fixture(project: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let artifact = port_dir().join("goldens").join("obl").join("Bank.obl.json");
        let gen_json = port_dir().join("goldens").join("gen").join("Bank.gen.json");
        let gen_lean = port_dir()
            .join("Tyu")
            .join("Gen")
            .join("Golden")
            .join("Bank.lean");
        assert!(artifact.is_file(), "missing {}", artifact.display());
        assert!(gen_json.is_file(), "missing {}", gen_json.display());
        assert!(gen_lean.is_file(), "missing {}", gen_lean.display());
        fs::create_dir_all(project.join("out")).unwrap();
        let _ = fs::copy(&artifact, project.join("out").join("Bank.obl.json")).unwrap();
        let gen_dir = project.join("gen");
        fs::create_dir_all(&gen_dir).unwrap();
        let _ = fs::copy(&gen_json, gen_dir.join("Bank.gen.json")).unwrap();
        let _ = fs::copy(&gen_lean, gen_dir.join("Bank.lean")).unwrap();
        (project.join("out").join("Bank.obl.json"), gen_dir, gen_lean)
    }

    #[test]
    fn vendored_library_tamper_is_fail_closed() {
        let dir = temp_dir("vendor");
        let project = dir.join("project");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(project.join("proofs")).unwrap();
        fs::write(project.join("proofs").join("proofs.lean"), b"").unwrap();
        let (artifact, _, _) = corpus_fixture(&project);
        let port = synthetic_port(&dir);
        let pkg = generate_package(&project, &port, &[artifact]).unwrap();
        // A clean package verifies and yields a digest.
        let digest = verify_vendored_files(&pkg, &port).unwrap();
        assert_eq!(digest.len(), 64, "sha256 hex");
        assert_eq!(
            verify_vendored_files(&pkg, &port).unwrap(),
            digest,
            "deterministic"
        );
        // Tamper one vendored semantics file → E6418, fail-closed.
        let step = pkg.root.join("Tyu").join("Step.lean");
        let text = fs::read_to_string(&step).unwrap();
        fs::write(&step, format!("-- tampered\n{text}")).unwrap();
        let err = verify_vendored_files(&pkg, &port).unwrap_err();
        assert!(err.to_string().contains("E6418"), "err: {err}");
        assert!(
            err.to_string().contains("tampered Tyu library"),
            "must name the failure: {err}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gen_digest_check_agrees_with_the_port_on_corpus_and_catches_tamper() {
        let dir = temp_dir("e6418");
        let (artifact, gen_dir, _) = corpus_fixture(&dir);
        let pkg = LeanPackage {
            root: dir.clone(),
            gen_dir: gen_dir.clone(),
            project_root: dir.clone(),
        };
        let statements = verify_gen_digests(&pkg, std::slice::from_ref(&artifact)).unwrap();
        // The Bank corpus: ≥ 1 rendered statement and ≥ 1 omitted one (the
        // opaque/contract rows the renderer refuses) — both arms exercised;
        // every rendered statement is unproven in P6.
        let bank = statements
            .iter()
            .find(|s| s.module == "Bank")
            .expect("Bank accounting present");
        assert!(bank.rendered >= 1, "rendered: {}", bank.rendered);
        assert!(bank.omitted >= 1, "omitted: {}", bank.omitted);
        assert_eq!(bank.unproven, bank.rendered, "P6: nothing proven yet");

        // Tamper the generated text's first statement hash — E6418, fail-closed.
        let lean_path = gen_dir.join("Bank.lean");
        let text = fs::read_to_string(&lean_path).unwrap();
        let real = extract_statement_hashes(&text).into_iter().next().unwrap();
        let tampered = format!("{}a", &real[..63]);
        let text2 = text.replacen(&real, &tampered, 1);
        assert_ne!(text2, text, "tamper must change the text");
        fs::write(&lean_path, text2).unwrap();
        let err = verify_gen_digests(&pkg, &[artifact]).unwrap_err();
        assert!(err.to_string().contains("E6418"), "err: {err}");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A synthetic port: the vendored files copied from the real port plus a
    /// fake `gen` renderer that copies the corpus golden output verbatim —
    /// hermetic package generation (no lake, no lean).
    fn synthetic_port(project: &Path) -> PathBuf {
        let port = project.join("port");
        for rel in vendored_port_files() {
            let src = port_dir().join(rel);
            let dst = port.join(rel);
            fs::create_dir_all(dst.parent().unwrap()).unwrap();
            fs::copy(&src, &dst).unwrap();
        }
        fs::create_dir_all(port.join(".lake").join("build").join("bin")).unwrap();
        // The fake gen renderer: `--render --obl <files> --out <dir>` copies
        // the corpus golden Bank surface into the out dir.
        let gen_dir_for = project.join("gen-src");
        fs::create_dir_all(&gen_dir_for).unwrap();
        fs::copy(
            port_dir().join("goldens").join("gen").join("Bank.gen.json"),
            gen_dir_for.join("Bank.gen.json"),
        )
        .unwrap();
        fs::copy(
            port_dir()
                .join("Tyu")
                .join("Gen")
                .join("Golden")
                .join("Bank.lean"),
            gen_dir_for.join("Bank.lean"),
        )
        .unwrap();
        let script = port.join(".lake").join("build").join("bin").join("gen");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\n\
                 out=\"\"\n\
                 prev=\"\"\n\
                 for a in \"$@\"; do\n\
                 \x20 if [ \"$prev\" = \"--out\" ]; then out=\"$a\"; fi\n\
                 \x20 prev=\"$a\"\n\
                 done\n\
                 cp \"{}/Bank.lean\" \"$out/Bank.lean\"\n\
                 cp \"{}/Bank.gen.json\" \"$out/Bank.gen.json\"\n",
                gen_dir_for.display(),
                gen_dir_for.display(),
            ),
        )
        .unwrap();
        // chmod +x
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).unwrap();
        }
        fs::write(port.join("lean-toolchain"), b"leanprover/lean4:v4.27.0\n").unwrap();
        port
    }

    #[test]
    fn generate_package_is_deterministic_and_fast_paths() {
        let dir = temp_dir("genpkg");
        let project = dir.join("project");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(project.join("proofs")).unwrap();
        fs::write(project.join("proofs").join("proofs.lean"), b"import Bank\n").unwrap();
        fs::write(
            project.join("proofs").join("Bank.lean"),
            b"import Gen.Bank\n-- no proofs yet\n",
        )
        .unwrap();
        let (artifact, _, _) = corpus_fixture(&project);

        let port = synthetic_port(&dir);
        let package1 = generate_package(&project, &port, std::slice::from_ref(&artifact)).unwrap();

        // Determinism: regenerating over the same inputs yields the identical
        // byte tree (a second generation fast-paths and does not rewrite).
        let state1 = fs::read(package1.root.join(".tyu-gen.json")).unwrap();
        let tree1 = tree_bytes(&package1.root);
        let package2 = generate_package(&project, &port, std::slice::from_ref(&artifact)).unwrap();
        assert_eq!(
            fs::read(package2.root.join(".tyu-gen.json")).unwrap(),
            state1,
            "state must be byte-stable"
        );
        assert_eq!(
            tree1,
            tree_bytes(&package2.root),
            "tree must be byte-stable"
        );

        // The lakefile resolves the developer proofs dir + enumerated roots,
        // and the harvest stub imports the proofs root.
        let lakefile = fs::read_to_string(package2.root.join("lakefile.toml")).unwrap();
        assert!(lakefile.contains("TyuProofs"), "lakefile: {lakefile}");
        assert!(
            lakefile.contains("\"Bank\", \"proofs\""),
            "roots: {lakefile}"
        );
        let stub = fs::read_to_string(package2.root.join("Harvest.lean")).unwrap();
        assert!(stub.contains("import proofs"), "stub: {stub}");

        // A proof-file change invalidates the fingerprint → the next call
        // regenerates (the state moves).
        let dir2 = temp_dir("genpkg2");
        fs::create_dir_all(dir2.join("proofs")).unwrap();
        fs::write(dir2.join("proofs").join("proofs.lean"), b"import Bank\n").unwrap();
        fs::write(
            dir2.join("proofs").join("Bank.lean"),
            b"import Gen.Bank\n-- changed\n",
        )
        .unwrap();
        let (artifact2, _, _) = corpus_fixture(&dir2);
        let package3 = generate_package(&dir2, &port, std::slice::from_ref(&artifact2)).unwrap();
        assert_ne!(
            fs::read(package3.root.join(".tyu-gen.json")).unwrap(),
            state1,
            "a proofs/ edit must change the generation state"
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    /// Recursively collect (relative path, bytes) for a directory, sorted —
    /// a byte-tree equality witness.
    fn tree_bytes(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            if let Ok(rd) = fs::read_dir(&d) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if let Ok(bytes) = fs::read(&p) {
                        if let Ok(rel) = p.strip_prefix(root) {
                            out.push((rel.to_string_lossy().into_owned(), bytes));
                        }
                    }
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn package_lock_waits_then_e6416_and_steals_stale_locks() {
        let dir = temp_dir("lock");
        fs::create_dir_all(&dir).unwrap();
        // A lock owned by a live PID (us) must block until the timeout → E6416.
        let lock = dir.join(".tyu-gen.lock");
        fs::write(&lock, format!("{}\n", std::process::id())).unwrap();
        std::env::set_var(LOCK_TIMEOUT_ENV, "300");
        let start = Instant::now();
        let err = lock_package(&dir).unwrap_err();
        assert!(err.to_string().contains("E6416"), "err: {err}");
        assert!(
            start.elapsed() >= Duration::from_millis(250),
            "must have waited: {:?}",
            start.elapsed()
        );
        std::env::remove_var(LOCK_TIMEOUT_ENV);

        // A lock owned by a dead PID is stolen immediately.
        fs::write(&lock, "99999999\n").unwrap();
        let guard = lock_package(&dir).unwrap();
        let held = fs::read_to_string(&lock).unwrap();
        assert!(
            held.trim() == std::process::id().to_string(),
            "lock must be re-held by us after the steal: {held:?}"
        );
        drop(guard);
        assert!(!lock.exists(), "dropping releases the lock");
        let _ = fs::remove_dir_all(&dir);
    }

    // -------------------------------------------------------------------
    // Toolchain E6416 paths (check_lean_toolchain is hermetic — a fake
    // `lean` script stands in for the version probe).
    // -------------------------------------------------------------------

    /// A fake `lean` binary that prints a given version (the shape
    /// `lean_version` parses).
    fn fake_lean(dir: &Path, version: &str) -> PathBuf {
        let bin = dir.join("lean");
        fs::write(
            &bin,
            format!(
                "#!/bin/sh\n\
                 if [ \"$1\" = \"--version\" ]; then\n\
                 \x20 echo \"Lean (version {version}, x86_64-pc-linux-gnu, Release)\"\n\
                 fi\n"
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&bin).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&bin, perms).unwrap();
        }
        bin
    }

    #[test]
    fn toolchain_missing_is_e6416_with_the_pin() {
        let err =
            check_lean_toolchain("leanprover/lean4:v4.27.0", "4.27.0", None, true).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("E6416"), "err: {msg}");
        assert!(msg.contains("Lean toolchain unavailable"), "err: {msg}");
        assert!(
            msg.contains("leanprover/lean4:v4.27.0"),
            "fail-closed with the pin visible: {msg}"
        );
    }

    #[test]
    fn toolchain_version_mismatch_is_e6416() {
        let dir = temp_dir("tcmis");
        let lean = fake_lean(&dir, "4.26.0");
        let err = check_lean_toolchain("leanprover/lean4:v4.27.0", "4.27.0", Some(&lean), true)
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("E6416"), "err: {msg}");
        assert!(msg.contains("Lean toolchain mismatch"), "err: {msg}");
        assert!(msg.contains("4.26.0"), "names the resolved version: {msg}");
        assert!(msg.contains("4.27.0"), "names the pin: {msg}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lake_missing_is_e6416() {
        let dir = temp_dir("lakeno");
        let lean = fake_lean(&dir, "4.27.0");
        let err = check_lean_toolchain("leanprover/lean4:v4.27.0", "4.27.0", Some(&lean), false)
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("E6416"), "err: {msg}");
        assert!(msg.contains("`lake` not found"), "err: {msg}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn toolchain_matching_pin_is_ok() {
        let dir = temp_dir("tcok");
        let lean = fake_lean(&dir, "4.27.0");
        check_lean_toolchain("leanprover/lean4:v4.27.0", "4.27.0", Some(&lean), true)
            .expect("matching pin + lake present must pass");
        let _ = fs::remove_dir_all(&dir);
    }

    // -------------------------------------------------------------------
    // §7.2 verdicts-slot key: naming, parsing, and environment sensitivity
    // -------------------------------------------------------------------

    fn sample_env() -> VerifyEnvKey {
        VerifyEnvKey {
            semantics: "tyu.ir-sem/1.0".to_string(),
            stmt: "tyu.stmt/1.0".to_string(),
            toolchain_hash: 0x1234_5678_9abc_def0,
            model_id: verifier::model::MODEL_UNMODELED.to_string(),
            proof_files_hash: 0x0bad_cafe_0bad_cafe,
        }
    }

    #[test]
    fn verdicts_slot_name_round_trips_and_is_env_sensitive() {
        let env = sample_env();
        let name = verdicts_slot_name("Bank", 0x1020_3040_5060_7080, &env);
        // The extended components are visible and sanitized (no `/`).
        assert!(name.starts_with("Bank-1020304050607080-"), "name: {name}");
        assert!(name.contains("-sem-tyu.ir-sem_1.0-"), "name: {name}");
        assert!(name.contains("-stmt-tyu.stmt_1.0-"), "name: {name}");
        assert!(name.contains("-tc-123456789abcdef0-"), "name: {name}");
        assert!(name.contains("-model-unmodeled-"), "name: {name}");
        assert!(
            name.contains("-proofs-0badcafe0badcafe.verdicts.json"),
            "name: {name}"
        );
        assert!(
            !name.contains('/'),
            "no path separators may enter a file name"
        );

        // The strict parse round-trips under the same environment…
        let parsed = parse_verdicts_slot_name(&name, &env).expect("parses under env");
        assert_eq!(parsed, ("Bank".to_string(), 0x1020_3040_5060_7080));

        // …and a changed proof environment (a proof-file edit) does NOT parse
        // as this build's slot: the stale-verdicts silent-reuse failure mode
        // is structurally impossible.
        let env2 = VerifyEnvKey {
            proof_files_hash: 0xdead_beef_dead_beef,
            ..env.clone()
        };
        assert!(
            parse_verdicts_slot_name(&name, &env2).is_none(),
            "a different proof environment must not satisfy this slot"
        );
        let name2 = verdicts_slot_name("Bank", 0x1020_3040_5060_7080, &env2);
        assert_ne!(name, name2);
        assert!(parse_verdicts_slot_name(&name2, &env2).is_some());
    }

    #[test]
    fn verdicts_slot_model_id_change_invalidates() {
        let env = sample_env();
        let env2 = VerifyEnvKey {
            model_id: "tyu.model/rp2350/1".to_string(),
            ..env.clone()
        };
        let n1 = verdicts_slot_name("App", 1, &env);
        let n2 = verdicts_slot_name("App", 1, &env2);
        assert_ne!(n1, n2, "a model-id change must rotate the slot");
        assert!(parse_verdicts_slot_name(&n1, &env).is_some());
        assert!(parse_verdicts_slot_name(&n1, &env2).is_none());
    }

    #[test]
    fn loose_slot_parse_handles_legacy_and_extended_names() {
        // Legacy short form (pre-§7.2) still identifies (module, fp).
        assert_eq!(
            parse_any_verdicts_name("Bank-1020304050607080.verdicts.json"),
            Some(("Bank".to_string(), 0x1020_3040_5060_7080))
        );
        // Extended form (any env-slug).
        let env = sample_env();
        let name = verdicts_slot_name("Bank", 0x1020_3040_5060_7080, &env);
        assert_eq!(
            parse_any_verdicts_name(&name),
            Some(("Bank".to_string(), 0x1020_3040_5060_7080))
        );
        // Non-slot files are refused.
        assert!(parse_any_verdicts_name("image-verdicts.json").is_none());
        assert!(parse_any_verdicts_name("Foo-zzzz.verdicts.json").is_none());
    }

    #[test]
    fn slot_sanitizer_escapes_slashes_only() {
        assert_eq!(sanitize_component("tyu.ir-sem/1.0"), "tyu.ir-sem_1.0");
        assert_eq!(sanitize_component("unmodeled"), "unmodeled");
    }
}
