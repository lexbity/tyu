//! Platform manifest model, discovery, and CLI reporting.

use crate::args::PlatformArgs;
use crate::error::TyuError;
use codegen_core::{PlatformCapability, Target};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use super::linker_script::scaffold_platform_pack;
use super::lint::{format_lint_outcome, lint_pack};

const WORKSPACE_ROOT: &str = env!("CARGO_MANIFEST_DIR");
const E_PACK_NAME_CONFLICT: u16 = 5409;

pub(crate) fn workspace_root() -> PathBuf {
    Path::new(WORKSPACE_ROOT)
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or_else(|| Path::new(WORKSPACE_ROOT))
        .to_path_buf()
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct PlatformManifest {
    pub platform: PlatformSection,
    pub metal: MetalSection,
    #[serde(default)]
    pub memory: Option<MemorySection>,
    #[serde(default)]
    pub features: HashMap<String, FeatureUnit>,
    #[serde(default)]
    pub capabilities: HashMap<String, CapabilityConfig>,
    #[serde(default)]
    pub deploy: Option<DeploySection>,
    #[serde(default)]
    pub debug: Option<DebugSection>,
    pub test: TestSection,
    #[serde(default)]
    pub secure_boot: Option<SecureBootSection>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct PlatformSection {
    pub name: String,
    #[serde(rename = "compiler-interface")]
    pub compiler_interface: u16,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub isa: Vec<IsaEntry>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct IsaEntry {
    pub triple: String,
    pub arch: String,
    #[serde(default)]
    pub default: bool,
    #[serde(default, rename = "expected_abi_hash")]
    pub expected_abi_hash: Option<String>,
    #[serde(default)]
    pub metal: Option<MetalSection>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct MetalSection {
    pub path: String,
    pub startup: String,
    pub linker: String,
    #[serde(default)]
    pub required_symbols: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct MemorySection {
    pub flash: Option<MemoryRegion>,
    #[serde(default)]
    pub sram: Option<MemoryRegion>,
    #[serde(default)]
    pub ds_region: Option<String>,
    #[serde(default)]
    pub ds_size: Option<u64>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct MemoryRegion {
    pub name: String,
    pub origin: u64,
    pub length: u64,
    #[serde(default)]
    pub exec: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct FeatureUnit {
    pub unit: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CapabilityConfig {
    pub glue: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct DeploySection {
    pub method: String,
    #[serde(default)]
    pub boot: String,
    #[serde(default, rename = "step")]
    pub steps: Vec<DeployStep>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct DeployStep {
    pub run: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct DebugSection {
    #[serde(rename = "diag_transport")]
    pub diag_transport: String,
    #[serde(default)]
    pub rsp: String,
    #[serde(default)]
    pub probe: String,
    #[serde(default)]
    pub probe_config: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct DebugAgentSection {
    #[serde(default)]
    pub supported: bool,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub evidence: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TestRung {
    Untested,
    Qemu,
    Hardware,
}

impl TestRung {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestRung::Untested => "untested",
            TestRung::Qemu => "qemu",
            TestRung::Hardware => "hardware",
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct TestSection {
    pub rung: TestRung,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub debug_agent: Option<DebugAgentSection>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct SecureBootSection {
    #[serde(default)]
    pub supported: bool,
    #[serde(default, rename = "encryption_implies_secure_boot")]
    pub encryption_implies_secure_boot: bool,
}

#[derive(Clone, Debug)]
pub struct PlatformPack {
    pub manifest_path: PathBuf,
    pub root: PathBuf,
    pub manifest: PlatformManifest,
}

#[derive(Clone, Debug)]
pub struct ResolvedPlatformSelection {
    pub pack: PlatformPack,
    pub isa: IsaEntry,
    pub target: Target,
}

impl PlatformPack {
    pub fn name(&self) -> &str {
        &self.manifest.platform.name
    }

    pub fn pack_root(&self) -> &Path {
        self.manifest_path.parent().unwrap_or(&self.root)
    }

    pub fn isa_summary(&self) -> String {
        if self.manifest.platform.isa.is_empty() {
            return "(none)".to_string();
        }
        self.manifest
            .platform
            .isa
            .iter()
            .map(|isa| {
                let mut label = format!("{} [{}]", isa.triple, isa.arch);
                if isa.default {
                    label.push_str(" *");
                }
                label
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn capabilities_summary(&self) -> String {
        if self.manifest.capabilities.is_empty() {
            return "(none)".to_string();
        }
        let mut keys: Vec<_> = self.manifest.capabilities.keys().cloned().collect();
        keys.sort();
        keys.join(", ")
    }

    pub fn display_path(&self) -> String {
        self.manifest_path
            .strip_prefix(&self.root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| self.manifest_path.display().to_string())
    }

    pub fn debug_summary(&self) -> String {
        match &self.manifest.debug {
            Some(debug) => format!(
                "diag_transport={} rsp={} probe={} probe_config={}",
                debug.diag_transport,
                empty_as_none(&debug.rsp),
                empty_as_none(&debug.probe),
                empty_as_none(&debug.probe_config),
            ),
            None => "(none)".to_string(),
        }
    }

    pub fn deploy_summary(&self) -> String {
        match &self.manifest.deploy {
            Some(deploy) => format!(
                "method={} boot={} steps={}",
                deploy.method,
                empty_as_none(&deploy.boot),
                deploy.steps.len()
            ),
            None => "(none)".to_string(),
        }
    }

    pub fn test_summary(&self) -> String {
        let target = self
            .manifest
            .test
            .target
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("(none)");
        let evidence = self
            .manifest
            .test
            .evidence
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("(none)");
        format!(
            "proven-rung={} target={} evidence={}",
            self.proven_test_rung_label(),
            target,
            evidence
        )
    }

    pub fn debug_agent_summary(&self) -> String {
        match self.manifest.test.debug_agent.as_ref() {
            Some(debug_agent) => format!(
                "supported={} target={} evidence={}",
                debug_agent.supported,
                debug_agent
                    .target
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("(none)"),
                debug_agent
                    .evidence
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("(none)"),
            ),
            None => "supported=false target=(none) evidence=(none)".to_string(),
        }
    }

    fn proven_test_rung_label(&self) -> &'static str {
        if !self.test_evidence_exists() {
            return "untested";
        }

        match self.manifest.test.rung {
            TestRung::Untested => "untested",
            TestRung::Qemu => "qemu (automated)",
            TestRung::Hardware => "hardware (manual)",
        }
    }

    fn test_evidence_exists(&self) -> bool {
        self.manifest
            .test
            .evidence
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|evidence| self.pack_root().join(evidence).exists())
            .unwrap_or(false)
    }

    pub fn effective_metal<'a>(&'a self, isa: &'a IsaEntry) -> &'a MetalSection {
        isa.metal.as_ref().unwrap_or(&self.manifest.metal)
    }
}

impl ResolvedPlatformSelection {
    pub fn metal(&self) -> &MetalSection {
        self.pack.effective_metal(&self.isa)
    }

    pub fn pack_root(&self) -> &Path {
        self.pack.pack_root()
    }
}

fn empty_as_none(text: &str) -> &str {
    if text.is_empty() {
        "(none)"
    } else {
        text
    }
}

pub fn run(args: PlatformArgs) -> Result<(), TyuError> {
    match args {
        PlatformArgs::List => {
            print_list(&workspace_root())?;
            Ok(())
        }
        PlatformArgs::Info { name, isa } => {
            print_info(&workspace_root(), &name, isa.as_deref())?;
            Ok(())
        }
        PlatformArgs::Lint { name, all } => {
            let outcome = lint_pack(&workspace_root(), &name, all)?;
            print!("{}", format_lint_outcome(&outcome));
            if outcome.errors.is_empty() {
                Ok(())
            } else {
                Err(TyuError::Platform(format!(
                    "lint failed with {} error(s)",
                    outcome.errors.len()
                )))
            }
        }
        PlatformArgs::New { name } => {
            let created = scaffold_platform_pack(&workspace_root(), &name)?;
            println!("created platform pack '{}': {}", name, created.display());
            Ok(())
        }
    }
}

pub fn print_list(root: &Path) -> Result<(), TyuError> {
    print!("{}", list_report(root)?);
    Ok(())
}

pub fn print_info(root: &Path, name: &str, isa_filter: Option<&str>) -> Result<(), TyuError> {
    print!("{}", info_report(root, name, isa_filter)?);
    Ok(())
}

pub fn list_report(root: &Path) -> Result<String, TyuError> {
    let packs = discover_platforms_in(root)?;
    let mut out = String::new();
    for pack in packs {
        writeln!(
            &mut out,
            "{} | isa={} | compiler-interface={} | capabilities={} | rung={}",
            pack.name(),
            pack.isa_summary(),
            pack.manifest.platform.compiler_interface,
            pack.capabilities_summary(),
            pack.manifest.test.rung.as_str(),
        )
        .map_err(|e| TyuError::Platform(e.to_string()))?;
    }
    Ok(out)
}

pub fn info_report(root: &Path, name: &str, isa_filter: Option<&str>) -> Result<String, TyuError> {
    let packs = discover_platforms_in(root)?;
    let pack = packs
        .iter()
        .find(|pack| pack.name() == name)
        .ok_or_else(|| TyuError::Platform(format!("platform pack '{}' not found", name)))?;

    if let Some(filter) = isa_filter {
        let matches = pack
            .manifest
            .platform
            .isa
            .iter()
            .any(|isa| isa.arch == filter || isa.triple == filter);
        if !matches {
            return Err(TyuError::Platform(format!(
                "platform pack '{}' does not declare isa '{}'",
                name, filter
            )));
        }
    }

    let mut out = String::new();
    writeln!(&mut out, "platform {}", pack.name())
        .map_err(|e| TyuError::Platform(e.to_string()))?;
    writeln!(&mut out, "  manifest: {}", pack.display_path())
        .map_err(|e| TyuError::Platform(e.to_string()))?;
    if let Some(desc) = pack
        .manifest
        .platform
        .description
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        writeln!(&mut out, "  description: {}", desc)
            .map_err(|e| TyuError::Platform(e.to_string()))?;
    }
    writeln!(
        &mut out,
        "  compiler-interface: {}",
        pack.manifest.platform.compiler_interface
    )
    .map_err(|e| TyuError::Platform(e.to_string()))?;
    writeln!(&mut out, "  isa: {}", pack.isa_summary())
        .map_err(|e| TyuError::Platform(e.to_string()))?;
    for isa in &pack.manifest.platform.isa {
        if isa.metal.is_some() {
            let metal = pack.effective_metal(isa);
            writeln!(
                &mut out,
                "  isa-metal[{}]: path={} startup={} linker={}",
                isa.triple,
                metal.path,
                metal.startup,
                empty_as_none(&metal.linker),
            )
            .map_err(|e| TyuError::Platform(e.to_string()))?;
        }
    }
    writeln!(
        &mut out,
        "  metal: path={} startup={} linker={}",
        pack.manifest.metal.path,
        pack.manifest.metal.startup,
        empty_as_none(&pack.manifest.metal.linker),
    )
    .map_err(|e| TyuError::Platform(e.to_string()))?;
    if !pack.manifest.metal.required_symbols.is_empty() {
        writeln!(
            &mut out,
            "  required-symbols: {}",
            pack.manifest.metal.required_symbols.join(", ")
        )
        .map_err(|e| TyuError::Platform(e.to_string()))?;
    }
    if !pack.manifest.features.is_empty() {
        let mut features: Vec<_> = pack.manifest.features.iter().collect();
        features.sort_by(|a, b| a.0.cmp(b.0));
        writeln!(
            &mut out,
            "  features: {}",
            features
                .into_iter()
                .map(|(name, unit)| format!("{}={}", name, unit.unit))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .map_err(|e| TyuError::Platform(e.to_string()))?;
    }
    writeln!(&mut out, "  capabilities: {}", pack.capabilities_summary())
        .map_err(|e| TyuError::Platform(e.to_string()))?;
    writeln!(&mut out, "  deploy: {}", pack.deploy_summary())
        .map_err(|e| TyuError::Platform(e.to_string()))?;
    writeln!(&mut out, "  debug: {}", pack.debug_summary())
        .map_err(|e| TyuError::Platform(e.to_string()))?;
    writeln!(&mut out, "  test: {}", pack.test_summary())
        .map_err(|e| TyuError::Platform(e.to_string()))?;
    writeln!(&mut out, "  debug-agent: {}", pack.debug_agent_summary())
        .map_err(|e| TyuError::Platform(e.to_string()))?;
    if let Some(secure_boot) = &pack.manifest.secure_boot {
        writeln!(
            &mut out,
            "  secure-boot: supported={} encryption_implies_secure_boot={}",
            secure_boot.supported, secure_boot.encryption_implies_secure_boot,
        )
        .map_err(|e| TyuError::Platform(e.to_string()))?;
    }
    Ok(out)
}

pub(crate) fn find_pack_manifest_path(root: &Path, name: &str) -> Option<PathBuf> {
    let candidates = [
        root.join("platforms").join(name).join("platform.toml"),
        root.join("runtime").join(name).join("platform.toml"),
        root.join("runtime").join(format!("{}.platform.toml", name)),
    ];
    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn discover_platforms() -> Result<Vec<PlatformPack>, TyuError> {
    discover_platforms_in(&workspace_root())
}

pub fn discover_platforms_in(root: &Path) -> Result<Vec<PlatformPack>, TyuError> {
    let mut packs: HashMap<String, PlatformPack> = HashMap::new();
    for manifest_path in discover_manifest_paths(root)? {
        let pack = load_platform_pack(root, &manifest_path)?;
        let name = pack.name().to_string();
        match packs.get(&name) {
            None => {
                packs.insert(name, pack);
            }
            Some(existing) => {
                let existing_prio = manifest_layout_priority(root, &existing.manifest_path);
                let new_prio = manifest_layout_priority(root, &pack.manifest_path);
                if new_prio < existing_prio {
                    packs.insert(name, pack);
                } else if new_prio == existing_prio {
                    return Err(TyuError::Platform(format!(
                        "E{} pack={} detail=duplicate manifest '{}' and '{}'",
                        E_PACK_NAME_CONFLICT,
                        name,
                        existing.manifest_path.display(),
                        pack.manifest_path.display()
                    )));
                }
            }
        }
    }
    let mut packs: Vec<PlatformPack> = packs.into_values().collect();
    packs.sort_by(|a, b| {
        a.name()
            .cmp(b.name())
            .then_with(|| a.manifest_path.cmp(&b.manifest_path))
    });
    Ok(packs)
}

fn manifest_layout_priority(root: &Path, manifest_path: &Path) -> u8 {
    let platforms_root = root.join("platforms");
    if manifest_path.starts_with(&platforms_root) {
        0
    } else if manifest_path.starts_with(root.join("runtime")) {
        1
    } else {
        2
    }
}

pub fn resolve_platform_selection(
    root: &Path,
    name: &str,
    isa_filter: Option<&str>,
) -> Result<ResolvedPlatformSelection, TyuError> {
    let pack = discover_platforms_in(root)?
        .into_iter()
        .find(|pack| pack.name() == name)
        .ok_or_else(|| TyuError::Platform(format!("platform pack '{}' not found", name)))?;

    let isa = if let Some(filter) = isa_filter {
        pack.manifest
            .platform
            .isa
            .iter()
            .find(|isa| isa.arch == filter || isa.triple == filter)
            .cloned()
            .ok_or_else(|| {
                TyuError::Platform(format!(
                    "platform pack '{}' does not declare isa '{}'",
                    name, filter
                ))
            })?
    } else {
        pack.manifest
            .platform
            .isa
            .iter()
            .find(|isa| isa.default)
            .cloned()
            .or_else(|| pack.manifest.platform.isa.first().cloned())
            .ok_or_else(|| {
                TyuError::Platform(format!("platform pack '{}' declares no isa", name))
            })?
    };

    let target = Target::parse(isa.triple.as_bytes())
        .ok_or_else(|| TyuError::Platform(format!("unknown target triple '{}'", isa.triple)))?;

    Ok(ResolvedPlatformSelection { pack, isa, target })
}

/// Derive runtime-service capabilities for a bare target by probing the
/// sysroot for the corresponding platform modules.
///
/// This is the single source of runtime-service capability resolution
/// (FR-12).  No per-target hard-coded match exists; every target resolves
/// by checking `sysroot/<triple>/platform/<svc>.mod` file presence.
pub fn capabilities_for_target(target: Target) -> HashSet<String> {
    let triple = std::str::from_utf8(target.triple()).unwrap_or("");
    let plat_dir = workspace_root()
        .join("sysroot")
        .join(triple)
        .join("platform");

    let mut caps = HashSet::new();

    // Channel IPC → Channels capability (hosted target only).
    if plat_dir.join("channel.mod").exists() {
        caps.insert(PlatformCapability::Channels.name().to_string());
    }
    // OS-level task scheduler → TaskScheduler capability (hosted target only).
    if plat_dir.join("linux.mod").exists() {
        caps.insert(PlatformCapability::TaskScheduler.name().to_string());
    }
    // Dynamic allocation is provided alongside these runtime services.
    if plat_dir.join("channel.mod").exists() || plat_dir.join("linux.mod").exists() {
        caps.insert(PlatformCapability::DynamicAlloc.name().to_string());
    }

    caps
}

pub fn capabilities_for_selection(selection: &ResolvedPlatformSelection) -> HashSet<String> {
    selection
        .pack
        .manifest
        .capabilities
        .keys()
        .cloned()
        .collect()
}

pub fn is_qemu_capable_selection(selection: &ResolvedPlatformSelection) -> bool {
    // QEMU-capable is a hardware fact: the resolved target has a QEMU machine.
    // It must NOT be conflated with the manifest's proven `test.rung`, which is
    // a separate honesty signal (ARM/RISC-V run in QEMU yet are still rung
    // "untested"). Gating capability on rung silently dropped real QEMU packs
    // from `--all-platforms`. See spec Vocabulary: QEMU-capable = qemu.is_some().
    selection.target.spec().qemu.is_some()
}

pub fn load_platform_pack(root: &Path, manifest_path: &Path) -> Result<PlatformPack, TyuError> {
    let text = fs::read_to_string(manifest_path)
        .map_err(|e| TyuError::Platform(format!("reading '{}': {}", manifest_path.display(), e)))?;
    let manifest: PlatformManifest = toml::from_str(&text)
        .map_err(|e| TyuError::Platform(format!("parsing '{}': {}", manifest_path.display(), e)))?;
    Ok(PlatformPack {
        manifest_path: manifest_path.to_path_buf(),
        root: root.to_path_buf(),
        manifest,
    })
}

fn discover_manifest_paths(root: &Path) -> Result<Vec<PathBuf>, TyuError> {
    let mut paths = Vec::new();
    let runtime = root.join("runtime");
    if runtime.is_dir() {
        for entry in fs::read_dir(&runtime)
            .map_err(|e| TyuError::Platform(format!("reading '{}': {}", runtime.display(), e)))?
        {
            let entry = entry.map_err(|e| {
                TyuError::Platform(format!("reading '{}': {}", runtime.display(), e))
            })?;
            let path = entry.path();
            if path.is_dir() {
                let candidate = path.join("platform.toml");
                if candidate.is_file() {
                    paths.push(candidate);
                }
            } else if is_platform_manifest_file(&path) {
                paths.push(path);
            }
        }
    }

    let platforms = root.join("platforms");
    if platforms.is_dir() {
        for entry in fs::read_dir(&platforms)
            .map_err(|e| TyuError::Platform(format!("reading '{}': {}", platforms.display(), e)))?
        {
            let entry = entry.map_err(|e| {
                TyuError::Platform(format!("reading '{}': {}", platforms.display(), e))
            })?;
            let path = entry.path();
            if path.is_dir() {
                let candidate = path.join("platform.toml");
                if candidate.is_file() {
                    paths.push(candidate);
                }
            }
        }
    }

    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn is_platform_manifest_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.ends_with(".platform.toml"))
        .unwrap_or(false)
}
