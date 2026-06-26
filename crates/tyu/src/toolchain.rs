//! Toolchain resolution and `tyu toolchain check` command.
//!
//! Resolves tool binaries (assembler, linker, QEMU) for a target using
//! a precedence chain: flag override → manifest → env var → PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

use codegen_core::Target;

use crate::project::ProjectManifest;

pub const RISCV_AS_CANDIDATES: &[&str] = &[
    "riscv32-elf-as",
    "riscv32-unknown-elf-as",
    "riscv64-unknown-elf-as",
    "riscv64-linux-gnu-as",
];
pub const RISCV_LD_CANDIDATES: &[&str] = &[
    "riscv32-elf-ld",
    "riscv32-unknown-elf-ld",
    "riscv64-unknown-elf-ld",
    "riscv64-linux-gnu-ld",
];
pub const RISCV_NM_CANDIDATES: &[&str] = &[
    "riscv32-elf-nm",
    "riscv32-unknown-elf-nm",
    "riscv64-unknown-elf-nm",
    "riscv64-linux-gnu-nm",
];

/// A resolved tool with its source and optional version.
#[derive(Debug)]
pub struct ResolvedTool {
    /// Path to the binary.
    pub path: PathBuf,
    /// Where it was resolved from.
    pub source: ToolSource,
    /// Version string (first line of `--version` output), if available.
    pub version: Option<String>,
}

/// Where a tool was resolved from.
#[derive(Debug)]
pub enum ToolSource {
    /// `--tool-<role>=<path>` flag.
    FlagOverride,
    /// `[toolchain.<triple>]` in `tyu.toml`.
    Manifest,
    /// `TYU_<ROLE>_<TRIPLE>` environment variable.
    EnvVar,
    /// `PATH` lookup of the target's default tool name.
    Path,
}

/// Roles that can be resolved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToolRole {
    Compiler,
    Assembler,
    Linker,
    Qemu,
}

impl ToolRole {
    pub fn default_name(self, target: Target) -> &'static [u8] {
        match self {
            ToolRole::Compiler => b"langc",
            ToolRole::Assembler => match target.spec().assembler {
                codegen_core::AssemblerKind::Fasm => b"fasm",
                codegen_core::AssemblerKind::GasArm => b"arm-none-eabi-as",
                codegen_core::AssemblerKind::GasRiscV => b"riscv32-elf-as",
            },
            ToolRole::Linker => target.spec().linker,
            ToolRole::Qemu => target.spec().qemu.map(|q| q.system_bin).unwrap_or(b""),
        }
    }

    pub fn env_var_name(self, target: Target) -> String {
        let triple = std::str::from_utf8(target.triple()).unwrap_or("unknown");
        let role = match self {
            ToolRole::Compiler => "COMPILER",
            ToolRole::Assembler => "AS",
            ToolRole::Linker => "LD",
            ToolRole::Qemu => "QEMU",
        };
        let triple_upper = triple.to_uppercase().replace('-', "_");
        format!("TYU_{}_{}", role, triple_upper)
    }
}

/// Result of resolving all tools for a target.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ToolResolution {
    pub target: Target,
    pub compiler: Option<ResolvedTool>,
    pub assembler: Option<ResolvedTool>,
    pub linker: Option<ResolvedTool>,
    pub qemu: Option<ResolvedTool>,
}

/// Resolve all tools for a target using the given manifest.
/// `flag_overrides` is a mapping from role name to explicit path.
pub fn resolve_tools(
    target: Target,
    manifest: &ProjectManifest,
    flag_overrides: &std::collections::HashMap<String, PathBuf>,
) -> ToolResolution {
    let triple = std::str::from_utf8(target.triple()).unwrap_or("");
    let tc = manifest.toolchain.get(triple);

    ToolResolution {
        target,
        compiler: resolve_one(target, ToolRole::Compiler, tc, flag_overrides),
        assembler: resolve_one(target, ToolRole::Assembler, tc, flag_overrides),
        linker: resolve_one(target, ToolRole::Linker, tc, flag_overrides),
        qemu: resolve_one(target, ToolRole::Qemu, tc, flag_overrides),
    }
}

fn resolve_one(
    target: Target,
    role: ToolRole,
    tc: Option<&crate::project::ToolchainConfig>,
    flag_overrides: &std::collections::HashMap<String, PathBuf>,
) -> Option<ResolvedTool> {
    let role_name = match role {
        ToolRole::Compiler => "langc",
        ToolRole::Assembler => "as",
        ToolRole::Linker => "ld",
        ToolRole::Qemu => "qemu",
    };

    // 1. Flag override: --tool-<role>=<path>
    if let Some(path) = flag_overrides.get(role_name) {
        return Some(ResolvedTool {
            path: path.clone(),
            source: ToolSource::FlagOverride,
            version: probe_version(path),
        });
    }

    // 2. Manifest: [toolchain.<triple>]
    if let Some(config) = tc {
        let path_str = match role {
            ToolRole::Compiler => None,
            ToolRole::Assembler => config.asm.as_ref(),
            ToolRole::Linker => config.ld.as_ref(),
            ToolRole::Qemu => config.qemu.as_ref(),
        };
        if let Some(p) = path_str {
            let path = PathBuf::from(p);
            let resolved_path = if path.is_file() {
                path.clone()
            } else if let Some(fp) = find_in_path(p) {
                fp
            } else {
                return None;
            };
            return Some(ResolvedTool {
                path: resolved_path,
                source: ToolSource::Manifest,
                version: probe_version(&path),
            });
        }
    }

    // 3. Environment variable: TYU_<ROLE>_<TRIPLE>
    let env_var = role.env_var_name(target);
    if let Ok(val) = std::env::var(&env_var) {
        let path = PathBuf::from(&val);
        let resolved_path = if path.is_file() {
            path.clone()
        } else if let Some(fp) = find_in_path(&val) {
            fp
        } else {
            return None;
        };
        return Some(ResolvedTool {
            path: resolved_path,
            source: ToolSource::EnvVar,
            version: probe_version(&path),
        });
    }

    // 4. PATH lookup of the default name.
    let default_name = role.default_name(target);
    if default_name.is_empty() {
        return None;
    }
    let name_str = std::str::from_utf8(default_name).ok()?;
    find_in_path(name_str).map(|path| ResolvedTool {
        path,
        source: ToolSource::Path,
        version: probe_version(&PathBuf::from(name_str)),
    })
}

/// Resolve a tool binary by name.
///
/// Checks, in order:
///   1. The workspace `target/debug/<name>` (development convenience).
///   2. `PATH` lookup.
///
/// Returns a clean `Err` if not found.
pub fn resolve_tool(name: &str) -> Result<PathBuf, String> {
    let candidates: &[&str] = match name {
        "riscv32-elf-as"
        | "riscv32-unknown-elf-as"
        | "riscv64-unknown-elf-as"
        | "riscv64-linux-gnu-as" => RISCV_AS_CANDIDATES,
        "riscv32-elf-ld"
        | "riscv32-unknown-elf-ld"
        | "riscv64-unknown-elf-ld"
        | "riscv64-linux-gnu-ld" => RISCV_LD_CANDIDATES,
        "riscv32-elf-nm"
        | "riscv32-unknown-elf-nm"
        | "riscv64-unknown-elf-nm"
        | "riscv64-linux-gnu-nm" => RISCV_NM_CANDIDATES,
        _ => &[name],
    };
    resolve_tool_candidates(candidates)
}

/// Resolve a tool binary from a list of acceptable candidate names.
pub fn resolve_tool_candidates(names: &[&str]) -> Result<PathBuf, String> {
    for name in names {
        // Development fallback: check workspace target/debug (avoids requiring
        // every contributor to add CARGO_TARGET_DIR/debug to their PATH).
        let workspace_debug = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("target")
            .join("debug")
            .join(name);
        if workspace_debug.is_file() {
            return Ok(workspace_debug);
        }

        if let Some(path) = find_in_path(name) {
            return Ok(path);
        }
    }

    Err(format!("tool '{}' not found in PATH", names.join(" or ")))
}

/// Find a binary — first in PATH, then in `target/debug/` (for workspace-built
/// tools like `langc` that are not on PATH but are built by `cargo build`).
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    // Check PATH.
    if let Some(p) = std::env::var_os("PATH").and_then(|path| {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    }) {
        return Some(p);
    }
    // Fall back to target/debug/ for workspace-built binaries.
    let ws = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .join("target")
        .join("debug")
        .join(name);
    if ws.is_file() {
        Some(ws)
    } else {
        None
    }
}

/// Probe a tool's version by running `<path> --version`.
fn probe_version(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().next().map(|s| s.to_string())
}

/// Run `tyu toolchain check <target>`.
/// Returns a human-readable report string.
pub fn toolchain_check(target: Target, manifest: &ProjectManifest) -> String {
    let flag_overrides = std::collections::HashMap::new();
    let res = resolve_tools(target, manifest, &flag_overrides);
    let triple = std::str::from_utf8(target.triple()).unwrap_or("???");

    let mut report = format!("Toolchain for {}:\n", triple);

    let mut add = |role: &str, tool: Option<&ResolvedTool>| match tool {
        Some(t) => {
            let src = match t.source {
                ToolSource::FlagOverride => "flag",
                ToolSource::Manifest => "manifest",
                ToolSource::EnvVar => "env",
                ToolSource::Path => "PATH",
            };
            let ver = t
                .version
                .as_ref()
                .map(|v| format!(" ({})", v))
                .unwrap_or_default();
            report.push_str(&format!(
                "  {}: found@{} [{}]{}\n",
                role,
                t.path.display(),
                src,
                ver
            ));
        }
        None => {
            report.push_str(&format!("  {}: MISSING\n", role));
        }
    };

    add("compiler", res.compiler.as_ref());
    add("assembler", res.assembler.as_ref());
    add("linker", res.linker.as_ref());
    add("qemu", res.qemu.as_ref());

    report
}
