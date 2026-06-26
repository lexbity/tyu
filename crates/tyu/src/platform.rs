//! Platform pack discovery and introspection for `tyu platform`.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::args::PlatformArgs;
use codegen_core::{PlatformCapability, Target};
use lmod::abi_hash::{compute_abi_hash, RUNTIME_ABI_VERSION};
use lmod::modinfo::MODINFO_VER;

const WORKSPACE_ROOT: &str = env!("CARGO_MANIFEST_DIR");
const MAX_PACK_FILE_BYTES: u64 = 64 * 1024;

const REQUIRED_BASE_SYMBOLS: &[&str] = &[
    "__lang_start",
    "__lang_trap",
    "__lang_ds_base",
    "__lang_ds_limit",
    "__lang_ds_high",
    "__lang_expected_abi_hash",
];

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
const E_PACK_NAME_CONFLICT: u16 = 5409;
const E_PACK_PATH_INVALID: u16 = 5410;
const E_PACK_FILE_TOO_LARGE: u16 = 5411;
const E_PACK_DEBUG_AGENT_UNBACKED: u16 = 5412;

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

pub fn run(args: PlatformArgs) -> Result<(), String> {
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
                Err(format!(
                    "lint failed with {} error(s)",
                    outcome.errors.len()
                ))
            }
        }
        PlatformArgs::New { name } => {
            let created = scaffold_platform_pack(&workspace_root(), &name)?;
            println!("created platform pack '{}': {}", name, created.display());
            Ok(())
        }
    }
}

pub fn print_list(root: &Path) -> Result<(), String> {
    print!("{}", list_report(root)?);
    Ok(())
}

pub fn print_info(root: &Path, name: &str, isa_filter: Option<&str>) -> Result<(), String> {
    print!("{}", info_report(root, name, isa_filter)?);
    Ok(())
}

pub fn list_report(root: &Path) -> Result<String, String> {
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
        .map_err(|e| e.to_string())?;
    }
    Ok(out)
}

pub fn info_report(root: &Path, name: &str, isa_filter: Option<&str>) -> Result<String, String> {
    let packs = discover_platforms_in(root)?;
    let pack = packs
        .iter()
        .find(|pack| pack.name() == name)
        .ok_or_else(|| format!("platform pack '{}' not found", name))?;

    if let Some(filter) = isa_filter {
        let matches = pack
            .manifest
            .platform
            .isa
            .iter()
            .any(|isa| isa.arch == filter || isa.triple == filter);
        if !matches {
            return Err(format!(
                "platform pack '{}' does not declare isa '{}'",
                name, filter
            ));
        }
    }

    let mut out = String::new();
    writeln!(&mut out, "platform {}", pack.name()).map_err(|e| e.to_string())?;
    writeln!(&mut out, "  manifest: {}", pack.display_path()).map_err(|e| e.to_string())?;
    if let Some(desc) = pack
        .manifest
        .platform
        .description
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        writeln!(&mut out, "  description: {}", desc).map_err(|e| e.to_string())?;
    }
    writeln!(
        &mut out,
        "  compiler-interface: {}",
        pack.manifest.platform.compiler_interface
    )
    .map_err(|e| e.to_string())?;
    writeln!(&mut out, "  isa: {}", pack.isa_summary()).map_err(|e| e.to_string())?;
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
            .map_err(|e| e.to_string())?;
        }
    }
    writeln!(
        &mut out,
        "  metal: path={} startup={} linker={}",
        pack.manifest.metal.path,
        pack.manifest.metal.startup,
        empty_as_none(&pack.manifest.metal.linker),
    )
    .map_err(|e| e.to_string())?;
    if !pack.manifest.metal.required_symbols.is_empty() {
        writeln!(
            &mut out,
            "  required-symbols: {}",
            pack.manifest.metal.required_symbols.join(", ")
        )
        .map_err(|e| e.to_string())?;
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
        .map_err(|e| e.to_string())?;
    }
    writeln!(&mut out, "  capabilities: {}", pack.capabilities_summary())
        .map_err(|e| e.to_string())?;
    writeln!(&mut out, "  deploy: {}", pack.deploy_summary()).map_err(|e| e.to_string())?;
    writeln!(&mut out, "  debug: {}", pack.debug_summary()).map_err(|e| e.to_string())?;
    writeln!(&mut out, "  test: {}", pack.test_summary()).map_err(|e| e.to_string())?;
    writeln!(&mut out, "  debug-agent: {}", pack.debug_agent_summary())
        .map_err(|e| e.to_string())?;
    if let Some(secure_boot) = &pack.manifest.secure_boot {
        writeln!(
            &mut out,
            "  secure-boot: supported={} encryption_implies_secure_boot={}",
            secure_boot.supported, secure_boot.encryption_implies_secure_boot,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(out)
}

pub fn lint_pack(root: &Path, name: &str, all: bool) -> Result<LintOutcome, String> {
    let manifest_path = find_pack_manifest_path(root, name)
        .ok_or_else(|| format!("platform pack '{}' not found", name))?;
    match load_platform_pack(root, &manifest_path) {
        Ok(pack) => lint_pack_manifest(root, &pack, all),
        Err(e) => Ok(LintOutcome {
            pack: name.to_string(),
            errors: vec![LintError::new(E_PACK_MANIFEST_INVALID, e)],
        }),
    }
}

pub fn scaffold_platform_pack(root: &Path, name: &str) -> Result<PathBuf, String> {
    if find_pack_manifest_path(root, name).is_some() {
        return Err(format!("platform pack '{}' already exists", name));
    }

    let pack_root = root.join("platforms").join(name);
    if pack_root.exists() {
        return Err(format!(
            "platform pack directory '{}' already exists",
            pack_root.display()
        ));
    }

    let metal_root = pack_root.join("metal");
    let glue_root = pack_root.join("glue");
    fs::create_dir_all(&metal_root)
        .map_err(|e| format!("creating '{}': {}", metal_root.display(), e))?;
    fs::create_dir_all(&glue_root)
        .map_err(|e| format!("creating '{}': {}", glue_root.display(), e))?;

    let target = Target::X86_64UnknownNone;
    let expected_abi_hash = compute_abi_hash(
        target.spec().calling_conv.arch_tag(),
        target.spec().slot_bytes,
        target.spec().word_bits,
        MODINFO_VER,
    );

    let manifest = scaffold_manifest(name, expected_abi_hash);
    fs::write(pack_root.join("platform.toml"), manifest).map_err(|e| {
        format!(
            "writing '{}': {}",
            pack_root.join("platform.toml").display(),
            e
        )
    })?;
    fs::write(metal_root.join("runtime.asm"), scaffold_runtime_asm()).map_err(|e| {
        format!(
            "writing '{}': {}",
            metal_root.join("runtime.asm").display(),
            e
        )
    })?;
    fs::write(metal_root.join("link.ld"), scaffold_linker_script())
        .map_err(|e| format!("writing '{}': {}", metal_root.join("link.ld").display(), e))?;
    fs::write(glue_root.join("gpio.def"), scaffold_glue_def("gpio"))
        .map_err(|e| format!("writing '{}': {}", glue_root.join("gpio.def").display(), e))?;
    fs::write(glue_root.join("gpio.mod"), scaffold_glue_mod("gpio"))
        .map_err(|e| format!("writing '{}': {}", glue_root.join("gpio.mod").display(), e))?;
    fs::write(glue_root.join("uart.def"), scaffold_glue_def("uart"))
        .map_err(|e| format!("writing '{}': {}", glue_root.join("uart.def").display(), e))?;
    fs::write(glue_root.join("uart.mod"), scaffold_glue_mod("uart"))
        .map_err(|e| format!("writing '{}': {}", glue_root.join("uart.mod").display(), e))?;
    fs::write(glue_root.join("time.def"), scaffold_glue_def("time"))
        .map_err(|e| format!("writing '{}': {}", glue_root.join("time.def").display(), e))?;
    fs::write(glue_root.join("time.mod"), scaffold_glue_mod("time"))
        .map_err(|e| format!("writing '{}': {}", glue_root.join("time.mod").display(), e))?;
    fs::write(pack_root.join("datasheet.md"), scaffold_datasheet(name)).map_err(|e| {
        format!(
            "writing '{}': {}",
            pack_root.join("datasheet.md").display(),
            e
        )
    })?;
    fs::write(pack_root.join("tier.md"), scaffold_tier(name))
        .map_err(|e| format!("writing '{}': {}", pack_root.join("tier.md").display(), e))?;

    Ok(pack_root)
}

fn scaffold_manifest(name: &str, expected_abi_hash: u64) -> String {
    format!(
        r#"[platform]
name = "{name}"
compiler-interface = {compiler_interface}
description = "{name} scaffold pack"

[[platform.isa]]
triple = "x86_64-unknown-none"
arch = "x86_64"
default = true
expected_abi_hash = "0x{expected_abi_hash:016x}"

[metal]
path = "metal"
startup = "runtime.asm"
linker = "link.ld"

[memory]
flash = {{ name = "FLASH", origin = 0x00000000, length = 0x00100000, exec = "xip" }}
sram = {{ name = "SRAM", origin = 0x20000000, length = 0x00010000 }}
ds_region = "SRAM"
ds_size = 0x4000

[deploy]
method = "elf-qemu"
boot = "raw_vectors"

[secure_boot]
supported = false
encryption_implies_secure_boot = true

[test]
rung = "untested"
"#,
        name = name,
        compiler_interface = RUNTIME_ABI_VERSION,
        expected_abi_hash = expected_abi_hash,
    )
}

fn scaffold_runtime_asm() -> String {
    [
        "format ELF64",
        "entry __lang_start",
        "",
        "section '.text' executable",
        "public __lang_start",
        "public __lang_trap",
        "",
        "__lang_start:",
        "    ret",
        "",
        "__lang_trap:",
        "    ret",
        "",
        "section '.data' writeable",
        "public __lang_ds_base",
        "public __lang_ds_limit",
        "public __lang_ds_high",
        "public __lang_expected_abi_hash",
        "",
        "__lang_ds_base:",
        "    dq 0",
        "__lang_ds_limit:",
        "    dq 0",
        "__lang_ds_high:",
        "    dq 0",
        "__lang_expected_abi_hash:",
        "    dq 0",
    ]
    .join("\n")
        + "\n"
}

fn scaffold_linker_script() -> String {
    [
        "ENTRY(__lang_start)",
        "MEMORY",
        "{",
        "    FLASH (rx) : ORIGIN = 0x00000000, LENGTH = 0x00100000",
        "    SRAM (rwx) : ORIGIN = 0x20000000, LENGTH = 0x00010000",
        "}",
        "SECTIONS",
        "{",
        "    .text : { *(.text*) } > FLASH",
        "    .rodata : { *(.rodata*) } > FLASH",
        "    .data : { *(.data*) } > SRAM",
        "    .bss : { *(.bss*) *(COMMON) } > SRAM",
        "}",
    ]
    .join("\n")
        + "\n"
}

fn scaffold_glue_def(capability: &str) -> String {
    let entries = match capability {
        "gpio" => vec![
            ("platform.gpio.init", "(pin mode --)"),
            ("platform.gpio.write", "(pin bool --)"),
            ("platform.gpio.read", "(pin -- bool)"),
        ],
        "uart" => vec![
            ("platform.uart.init", "(baud --)"),
            ("platform.uart.tx", "(u8 --)"),
            ("platform.uart.rx", "(-- u8 ok)"),
        ],
        "time" => vec![
            ("platform.time.now_us", "(-- i64)"),
            ("platform.time.reboot", "(--)"),
        ],
        _ => Vec::new(),
    };
    let mut out = String::new();
    for (name, effect) in entries {
        let _ = writeln!(&mut out, ": {} {} performs {{mmio}} ;", name, effect);
    }
    out
}

fn scaffold_glue_mod(capability: &str) -> String {
    format!(
        "module platform.{capability};\n\
         : stub ( -- ) ;\n\
         export {{ stub }};\n\
         end;\n",
    )
}

fn scaffold_datasheet(name: &str) -> String {
    format!(
        "# {name}\n\n\
         Scaffold pack generated by `tyu platform new`.\n\
         \n\
         This pack is intentionally minimal and should be filled in with the\n\
         board-specific memory map, boot flow, and peripheral details.\n",
    )
}

fn scaffold_tier(name: &str) -> String {
    format!(
        "# TestRung\n\n\
         TestRung = untested\n\n\
         Pack: {name}\n",
    )
}

/// Validate the platform pack selected for `target` against the runtime ABI.
///
/// This is the build-time gate for phase 4. It must run before codegen/tool
/// resolution so a stale pack fails fast with `E5401`.
pub fn ensure_build_platform_interface(root: &Path, target: Target) -> Result<(), String> {
    let triple =
        std::str::from_utf8(target.triple()).map_err(|_| "non-UTF-8 target triple".to_string())?;
    let packs = discover_platforms_in(root)?;

    if let Some(pack) = packs.iter().find(|pack| {
        pack.manifest
            .platform
            .isa
            .iter()
            .any(|isa| isa.triple == triple)
    }) {
        if pack.manifest.platform.compiler_interface != RUNTIME_ABI_VERSION as u16 {
            return Err(format!(
                "E{} pack={} isa={} detail=compiler-interface={} runtime-abi={}",
                E_PACK_INTERFACE_MISMATCH,
                pack.name(),
                triple,
                pack.manifest.platform.compiler_interface,
                RUNTIME_ABI_VERSION,
            ));
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

fn lint_pack_manifest(root: &Path, pack: &PlatformPack, all: bool) -> Result<LintOutcome, String> {
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
        .map_err(|detail| format!("{}: {}", pack.name(), detail))?;

    for isa in &manifest.platform.isa {
        let metal = pack.effective_metal(isa);
        let metal_root = validate_relative_path(pack_root, &metal.path)
            .map_err(|detail| format!("{}: {}", pack.name(), detail))?;
        let startup_rel = validate_relative_path(&metal_root, &metal.startup)
            .map_err(|detail| format!("{}: {}", pack.name(), detail))?;
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
                .map_err(|detail| format!("{}: {}", pack.name(), detail))?;
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

        let startup_text = fs::read_to_string(&startup_rel)
            .map_err(|e| format!("reading '{}': {}", startup_rel.display(), e))?;
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
                fs::read_to_string(linker_path)
                    .map_err(|e| format!("reading '{}': {}", linker_path.display(), e))?
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
            .map_err(|detail| format!("{}: {}", pack.name(), detail))?;
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
            .ok_or_else(|| format!("unknown target triple '{}'", isa.triple))?;
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
                    .map_err(|detail| format!("{}: {}", pack.name(), detail))?;
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
        if !matches!(deploy.method.as_str(), "elf-qemu" | "uf2" | "openocd") {
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
        .map_err(|detail| LintError::new(E_PACK_PATH_INVALID, detail))?;
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

fn validate_relative_path(root: &Path, rel: &str) -> Result<PathBuf, String> {
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
        return Err(format!("path '{}' escapes pack root", rel));
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

fn find_pack_manifest_path(root: &Path, name: &str) -> Option<PathBuf> {
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

pub fn discover_platforms() -> Result<Vec<PlatformPack>, String> {
    discover_platforms_in(&workspace_root())
}

pub fn discover_platforms_in(root: &Path) -> Result<Vec<PlatformPack>, String> {
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
                    return Err(format!(
                        "E{} pack={} detail=duplicate manifest '{}' and '{}'",
                        E_PACK_NAME_CONFLICT,
                        name,
                        existing.manifest_path.display(),
                        pack.manifest_path.display()
                    ));
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
) -> Result<ResolvedPlatformSelection, String> {
    let pack = discover_platforms_in(root)?
        .into_iter()
        .find(|pack| pack.name() == name)
        .ok_or_else(|| format!("platform pack '{}' not found", name))?;

    let isa = if let Some(filter) = isa_filter {
        pack.manifest
            .platform
            .isa
            .iter()
            .find(|isa| isa.arch == filter || isa.triple == filter)
            .cloned()
            .ok_or_else(|| format!("platform pack '{}' does not declare isa '{}'", name, filter))?
    } else {
        pack.manifest
            .platform
            .isa
            .iter()
            .find(|isa| isa.default)
            .cloned()
            .or_else(|| pack.manifest.platform.isa.first().cloned())
            .ok_or_else(|| format!("platform pack '{}' declares no isa", name))?
    };

    let target = Target::parse(isa.triple.as_bytes())
        .ok_or_else(|| format!("unknown target triple '{}'", isa.triple))?;

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

pub fn load_platform_pack(root: &Path, manifest_path: &Path) -> Result<PlatformPack, String> {
    let text = fs::read_to_string(manifest_path)
        .map_err(|e| format!("reading '{}': {}", manifest_path.display(), e))?;
    let manifest: PlatformManifest = toml::from_str(&text)
        .map_err(|e| format!("parsing '{}': {}", manifest_path.display(), e))?;
    Ok(PlatformPack {
        manifest_path: manifest_path.to_path_buf(),
        root: root.to_path_buf(),
        manifest,
    })
}

fn discover_manifest_paths(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    let runtime = root.join("runtime");
    if runtime.is_dir() {
        for entry in
            fs::read_dir(&runtime).map_err(|e| format!("reading '{}': {}", runtime.display(), e))?
        {
            let entry = entry.map_err(|e| format!("reading '{}': {}", runtime.display(), e))?;
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
            .map_err(|e| format!("reading '{}': {}", platforms.display(), e))?
        {
            let entry = entry.map_err(|e| format!("reading '{}': {}", platforms.display(), e))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_from_str(toml: &str) -> PlatformManifest {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn parses_basic_manifest() {
        let manifest = manifest_from_str(
            r#"
[platform]
name = "demo"
compiler-interface = 1

[[platform.isa]]
triple = "armv7m-unknown-none"
arch = "arm"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[test]
rung = "untested"
"#,
        );
        assert_eq!(manifest.platform.name, "demo");
        assert_eq!(manifest.platform.compiler_interface, 1);
        assert_eq!(manifest.platform.isa.len(), 1);
        assert_eq!(manifest.platform.isa[0].triple, "armv7m-unknown-none");
        assert_eq!(manifest.metal.startup, "runtime.asm");
        assert_eq!(manifest.test.rung, TestRung::Untested);
    }

    #[test]
    fn renders_summaries() {
        let root = std::env::temp_dir().join("tyu_platform_pack_summary");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("runtime/demo/tests")).unwrap();
        fs::write(root.join("runtime/demo/tests/demo.rs"), "evidence").unwrap();

        let manifest = manifest_from_str(
            r#"
[platform]
name = "demo"
compiler-interface = 1
description = "Demo pack"

[[platform.isa]]
triple = "armv7m-unknown-none"
arch = "arm"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[features.concurrency]
unit = "concurrency.asm"

[capabilities.uart]
glue = "glue/uart"

[deploy]
method = "elf-qemu"
boot = "raw_vectors"

[[deploy.step]]
run = "picotool"
args = ["load"]

[debug]
diag_transport = "semihosting"
rsp = "probe"
probe = "openocd"
probe_config = "demo.cfg"

[test]
rung = "qemu"
target = "crates/tyu/tests/run_qemu_x86.rs"
evidence = "tests/demo.rs"

[test.debug_agent]
supported = true
target = "crates/tyu/tests/escalate.rs"
evidence = "tests/demo.rs"
"#,
        );
        let pack = PlatformPack {
            manifest_path: root.join("runtime/demo/platform.toml"),
            root: root.join("runtime"),
            manifest,
        };
        assert!(pack.isa_summary().contains("armv7m-unknown-none"));
        assert_eq!(pack.capabilities_summary(), "uart");
        assert!(pack.deploy_summary().contains("method=elf-qemu"));
        assert!(pack.debug_summary().contains("diag_transport=semihosting"));
        assert!(pack.test_summary().contains("proven-rung=qemu (automated)"));
        assert!(pack
            .test_summary()
            .contains("target=crates/tyu/tests/run_qemu_x86.rs"));
        assert!(pack.test_summary().contains("evidence=tests/demo.rs"));
        assert!(pack.debug_agent_summary().contains("supported=true"));
        assert!(pack
            .debug_agent_summary()
            .contains("target=crates/tyu/tests/escalate.rs"));
        assert!(pack
            .debug_agent_summary()
            .contains("evidence=tests/demo.rs"));
    }

    #[test]
    fn scaffolds_platform_pack_and_refuses_overwrite() {
        let root = std::env::temp_dir().join("tyu_platform_scaffold");
        let _ = fs::remove_dir_all(&root);

        let pack_root = scaffold_platform_pack(&root, "demo").unwrap();
        assert_eq!(pack_root, root.join("platforms/demo"));
        assert!(pack_root.join("platform.toml").is_file());
        assert!(pack_root.join("metal/runtime.asm").is_file());
        assert!(pack_root.join("metal/link.ld").is_file());
        assert!(pack_root.join("glue/gpio.def").is_file());
        assert!(pack_root.join("glue/gpio.mod").is_file());
        assert!(pack_root.join("glue/uart.def").is_file());
        assert!(pack_root.join("glue/time.def").is_file());
        assert!(pack_root.join("datasheet.md").is_file());
        assert!(pack_root.join("tier.md").is_file());

        let tier = fs::read_to_string(pack_root.join("tier.md")).unwrap();
        assert!(tier.contains("TestRung = untested"));
        let manifest = fs::read_to_string(pack_root.join("platform.toml")).unwrap();
        assert!(manifest.contains("supported = false"));

        let lint = lint_pack(&root, "demo", false).unwrap();
        assert!(lint.errors.is_empty(), "{}", format_lint_outcome(&lint));

        let err = scaffold_platform_pack(&root, "demo").unwrap_err();
        assert!(err.contains("already exists"));
    }

    #[test]
    fn discovers_platform_files() {
        let root = std::env::temp_dir().join("tyu_platform_discovery");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("runtime/foo")).unwrap();
        fs::create_dir_all(root.join("platforms/bar")).unwrap();
        fs::write(
            root.join("runtime/foo/platform.toml"),
            r#"
[platform]
name = "foo"
compiler-interface = 1

[[platform.isa]]
triple = "x86_64-unknown-none"
arch = "x86"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[test]
rung = "untested"
"#,
        )
        .unwrap();
        fs::write(
            root.join("runtime/legacy.platform.toml"),
            r#"
[platform]
name = "legacy"
compiler-interface = 1

[[platform.isa]]
triple = "armv7m-unknown-none"
arch = "arm"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[test]
rung = "untested"
"#,
        )
        .unwrap();
        fs::write(
            root.join("platforms/bar/platform.toml"),
            r#"
[platform]
name = "bar"
compiler-interface = 1

[[platform.isa]]
triple = "riscv32-unknown-none"
arch = "riscv"
default = true

[metal]
path = "."
startup = "runtime.asm"
linker = "link.ld"

[test]
rung = "untested"
"#,
        )
        .unwrap();

        let packs = discover_platforms_in(&root).unwrap();
        let names: Vec<_> = packs.iter().map(|pack| pack.name().to_string()).collect();
        assert_eq!(names, vec!["bar", "foo", "legacy"]);
    }
}
