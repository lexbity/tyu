//! Toolchain resolution and `tyu toolchain check` command.
//!
//! Resolves tool binaries (assembler, linker, QEMU) for a target using
//! a precedence chain: flag override → manifest → env var → PATH.

use crate::error::TyuError;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
#[derive(Debug, Clone, Copy)]
pub enum ToolSource {
    /// `--tool-<role>=<path>` flag.
    FlagOverride,
    /// `[toolchain.<triple>]` in `tyu.toml`.
    Manifest,
    /// `TYU_<ROLE>_<TRIPLE>` environment variable.
    EnvVar,
    /// The workspace `target/debug/` dev fallback (matches what builds use).
    Workspace,
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
            } else {
                find_in_path(p)?
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
        } else {
            find_in_path(&val)?
        };
        return Some(ResolvedTool {
            path: resolved_path,
            source: ToolSource::EnvVar,
            version: probe_version(&path),
        });
    }

    // 4. Workspace target fallback (FR-13 runtime chain: running-exe dir →
    //    cwd/target/{debug,release}).  Builds resolve tools through
    //    `resolve_tool`/`resolve_tool_candidates`, which prefer the workspace
    //    binary over PATH.  Mirror that here so `toolchain check` reports the
    //    same binary the build driver actually uses. The chain is computed at
    //    RUNTIME (never `env!("CARGO_MANIFEST_DIR")`) so the binaries work
    //    from an installed prefix, a checkout, or a CWD-relative target dir.
    let default_name = role.default_name(target);
    if !default_name.is_empty() {
        let name_str = std::str::from_utf8(default_name).ok()?;
        if let Some(ws) = workspace_bin_in_target_dirs(name_str) {
            return Some(ResolvedTool {
                version: probe_version(&ws),
                path: ws,
                source: ToolSource::Workspace,
            });
        }
    }

    // 5. PATH lookup of the default name.
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
pub fn resolve_tool(name: &str) -> Result<PathBuf, TyuError> {
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
pub fn resolve_tool_candidates(names: &[&str]) -> Result<PathBuf, TyuError> {
    for name in names {
        // Worktree fallback (FR-13 runtime chain): check the running-exe dir
        // and cwd/target/{debug,release} (avoids requiring every contributor
        // to add target*/debug to their PATH). Runtime, never env!()-baked.
        if let Some(path) = workspace_bin_in_target_dirs(name) {
            return Ok(path);
        }

        if let Some(path) = find_in_path(name) {
            return Ok(path);
        }
    }

    Err(TyuError::Toolchain(format!(
        "tool '{}' not found in PATH",
        names.join(" or ")
    )))
}

/// Find a binary — first in PATH, then in the runtime workspace target dirs.
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
    // Fall back to the runtime workspace target dirs (FR-13).
    workspace_bin_in_target_dirs(name)
}

/// Runtime workspace-target lookup (FR-13): replaces the compile-time
/// `env!("CARGO_MANIFEST_DIR")` fallback, with PATH as the caller's final
/// rung. Chain in order:
///
///   1. the running executable's directory — covers `target/{debug,release}`
///      siblings (a release-built `tyu` finds its release siblings) and any
///      future installed layout (e.g. `$prefix/bin`); then
///   2. `cwd/target/{debug,release}`, walking up a bounded number of
///      ancestors — so a process running from a crate subdir (e.g. a test
///      binary in `crates/tooling-tests`) still resolves the WORKSPACE-root
///      target without a compile-time-baked path.
fn workspace_bin_in_target_dirs(name: &str) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()));
    let cwd = std::env::current_dir().ok();
    workspace_bin_in_dirs(exe_dir.as_deref(), cwd.as_deref(), name)
}

/// FR-13 testable core: running-exe dir, then cwd/ancestors'
/// target/{debug,release}.
fn workspace_bin_in_dirs(
    exe_dir: Option<&Path>,
    cwd: Option<&Path>,
    name: &str,
) -> Option<PathBuf> {
    if let Some(dir) = exe_dir {
        let sibling = dir.join(name);
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    if let Some(cwd) = cwd {
        for ancestor in cwd.ancestors().take(6) {
            for sub in ["debug", "release"] {
                let candidate = ancestor.join("target").join(sub).join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Probe a tool's version, dispatching per-tool (§6.2 probe table) — regex-
/// free token scans, a 1500 ms deadline, and a 4 KiB output cap, so a wedged
/// tool can never hang resolution or doctor.
pub(crate) fn probe_version(path: &Path) -> Option<String> {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // langc/tyu: presence only, never a version probe (§6.2).
    if name.starts_with("langc") || name == "tyu" {
        return None;
    }
    // fasm does not support `--version` — probe with no args; different fasm
    // builds print the version banner on stdout or stderr, so read both.
    let stream = if name == "fasm" {
        ProbeStream::Both
    } else {
        ProbeStream::Stdout
    };
    let args: &[&str] = if name == "fasm" { &[] } else { &["--version"] };
    let output = run_probe(path, args, stream)?;
    parse_probe_version(&name, &output)
}

/// Which stream a probe reads.
pub(crate) enum ProbeStream {
    Stdout,
    /// Both — fasm builds print the version banner on different streams.
    Both,
}

/// Run a probe with the shared deadline + output cap (§6.2). Returns `None`
/// on spawn failure, deadline kill, or zero captured bytes.
pub(crate) fn run_probe(bin: &Path, args: &[&str], stream: ProbeStream) -> Option<String> {
    const DEADLINE: Duration = Duration::from_millis(1500);
    const CAP: usize = 4096;

    let mut child = None;
    for attempt in 0..3 {
        match Command::new(bin)
            .args(args)
            // The probe is its own process group so a deadline kill can reap
            // the whole tree (a direct child kill leaves grandchildren
            // holding the pipe open).
            .process_group(0)
            .stdout(Stdio::piped())
            .stderr(if matches!(stream, ProbeStream::Both) {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .spawn()
        {
            Ok(c) => {
                child = Some(c);
                break;
            }
            Err(e) => {
                // ETXTBSY ("Text file busy", exec(2)) is transient: the image
                // was just written and another writer holds it — e.g. a fresh
                // fake tool in a test, or an installer atomically replacing a
                // binary. Retry briefly before failing.
                if e.raw_os_error() == Some(26) && attempt < 2 {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                return None;
            }
        }
    }
    let mut child = child?;
    let mut primary: Box<dyn Read + Send> = Box::new(child.stdout.take()?);
    let secondary_reader: Option<Box<dyn Read + Send>> = if matches!(stream, ProbeStream::Both) {
        Some(Box::new(child.stderr.take()?))
    } else {
        None
    };

    // Drain the primary pipe in a reader thread: `read` on a live pipe writer
    // blocks, which would defeat the deadline poll. The thread sees EOF when
    // the child exits (normally or via our kill), so join() returns promptly.
    let handle = std::thread::spawn(move || {
        let mut out: Vec<u8> = Vec::with_capacity(1024);
        let mut buf = [0u8; 2048];
        loop {
            match primary.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    out.extend_from_slice(&buf[..n]);
                    if out.len() >= CAP {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        // If we also captured stderr, drain it now (the child has exited, so
        // reads reach EOF immediately — no deadline hazard).
        if let Some(mut sec) = secondary_reader {
            let mut sec_buf = [0u8; 2048];
            loop {
                match sec.read(&mut sec_buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if out.len() >= CAP {
                            break;
                        }
                        let room = CAP - out.len();
                        out.extend_from_slice(&sec_buf[..n.min(room)]);
                    }
                    Err(_) => break,
                }
            }
        }
        out.truncate(CAP);
        out
    });

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let out_match = handle.join();
                let out = match out_match {
                    Ok(o) => o,
                    Err(_) => return None,
                };
                if out.is_empty() {
                    return None;
                }
                return Some(String::from_utf8_lossy(&out).into_owned());
            }
            Ok(None) => {
                if start.elapsed() >= DEADLINE {
                    // Kill the whole process group so pipe-holding
                    // grandchildren die too, then reap. The reader thread
                    // sees EOF and returns; join is bounded.
                    kill_process_group(&mut child);
                    let _ = child.wait();
                    let _ = handle.join();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return None,
        }
    }
}

/// Send SIGKILL to the probe's process group (set via `.process_group(0)`).
/// std::process::Child::kill targets only the direct child; a pipe-holding
/// grandchild would otherwise keep the reader thread pinned. The group kill
/// closes the pipe so the bounded join below returns.
fn kill_process_group(child: &mut std::process::Child) {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    // SAFETY: kill(-pgid, SIGKILL) with pgid == the child's pid (it is its
    // own process-group leader); valid for any child we spawned.
    unsafe {
        let _ = kill(-(child.id() as i32), 9); // SIGKILL
    }
    let _ = child.kill(); // belt and braces if the group kill missed
}

/// Extract a version from probe output based on the tool name (§6.2 table),
/// regex-free: `split_whitespace` + token scans on known prefixes.
fn parse_probe_version(name: &str, output: &str) -> Option<String> {
    let words: Vec<&str> = output.split_whitespace().collect();
    if name == "fasm" {
        // stderr: "flat assembler  version 1.73.32  (16384 kilobytes memory)"
        let i = words.iter().position(|w| *w == "version")?;
        return words.get(i + 1).map(|s| (*s).to_string());
    }
    if words.first().copied() == Some("rustc") {
        // stdout: "rustc 1.101.0-nightly (c1070d693 2026-09-28)"
        return words.get(1).map(|s| (*s).to_string());
    }
    if name.starts_with("qemu") {
        // stdout: "QEMU emulator version 8.2.2 (...)"
        for (i, w) in words.iter().enumerate() {
            if *w == "version" {
                return words
                    .get(i + 1)
                    .map(|s| (*s).trim_end_matches(',').to_string());
            }
        }
        return None;
    }
    if name == "lean" {
        // stdout: "Lean (version 4.16.0, commit ...)" — note the "(version"
        // token; trim the paren before comparing.
        for (i, w) in words.iter().enumerate() {
            if w.trim_start_matches('(') == "version" {
                return words
                    .get(i + 1)
                    .map(|s| (*s).trim_end_matches(',').to_string());
            }
        }
        return None;
    }
    // default (ld, as, nm, gcc-*, elan, …): first line verbatim (§6.2).
    output.lines().next().map(|s| s.to_string())
}

/// The human source label for a [`ToolSource`] — SHARED by `toolchain check`
/// and the doctor role report (FR-22 consolidation: one mapping, two layout
/// surfaces).
pub fn source_name(source: ToolSource) -> &'static str {
    match source {
        ToolSource::FlagOverride => "flag",
        ToolSource::Manifest => "manifest",
        ToolSource::EnvVar => "env",
        ToolSource::Workspace => "workspace target dir",
        ToolSource::Path => "PATH",
    }
}

/// Run `tyu toolchain check <target>`.
/// Returns a human-readable report string.
pub fn toolchain_check(target: Target, manifest: &ProjectManifest) -> String {
    let flag_overrides = std::collections::HashMap::new();
    let res = resolve_tools(target, manifest, &flag_overrides);
    render_toolchain_check(&res)
}

/// The byte-stable `toolchain check` layout, rendered from a `ToolResolution`
/// — the same resolution object `doctor::check_tools` produces (FR-12/FR-22:
/// one resolution, two renderers). The format is locked by
/// `toolchain_check_layout_is_byte_stable` in this module's tests.
pub fn render_toolchain_check(res: &ToolResolution) -> String {
    let triple = std::str::from_utf8(res.target.triple()).unwrap_or("???");
    let mut report = format!("Toolchain for {}:\n", triple);

    let mut add = |role: &str, tool: Option<&ResolvedTool>| match tool {
        Some(t) => {
            let src = source_name(t.source);
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

/// Test-only: serialize every test that reads the ambient `PATH` or spawns a
/// process by name. The doctor/toolchain tests fabricate PATHs to probe
/// resolution; concurrent by-name spawns in deploy/proof tests would see the
/// fabricated PATH and fail. All such tests acquire this single lock so
/// neither side ever observes the other's PATH.
#[cfg(test)]
pub(crate) fn ambient_path_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    /// A unique temp dir under the OS temp path.
    fn temp_dir(tag: &str) -> PathBuf {
        let unique = format!(
            "tyu-toolchain-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    /// Write an executable probe script.
    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&p, perms).unwrap();
        p
    }

    // --- FR-13 resolution order -------------------------------------------
    #[test]
    fn runtime_chain_prefers_exe_dir_then_cwd_targets() {
        let base = temp_dir("chain");
        let exe_dir = base.join("bin");
        let fake = write_script(&exe_dir, "fake-sibling", "#!/bin/sh\nexit 0\n");

        // exe-dir wins over a cwd/target candidate.
        let cwd = base.join("checkout");
        fs::create_dir_all(cwd.join("target").join("debug")).unwrap();
        fs::write(
            cwd.join("target").join("debug").join("fake-sibling"),
            b"other\n",
        )
        .unwrap();
        let got = workspace_bin_in_dirs(Some(&exe_dir), Some(&cwd), "fake-sibling");
        assert_eq!(got, Some(fake));

        // without an exe-dir sibling, cwd/target/debug is second.
        let exe_dir2 = base.join("empty-bin");
        fs::create_dir_all(&exe_dir2).unwrap();
        let got2 = workspace_bin_in_dirs(Some(&exe_dir2), Some(&cwd), "fake-sibling");
        assert_eq!(
            got2,
            Some(cwd.join("target").join("debug").join("fake-sibling"))
        );

        // release is third, after debug.
        fs::create_dir_all(cwd.join("target").join("release")).unwrap();
        fs::write(
            cwd.join("target").join("release").join("fake-sibling"),
            b"rel\n",
        )
        .unwrap();
        assert_eq!(
            workspace_bin_in_dirs(Some(&exe_dir2), Some(&cwd), "fake-sibling"),
            Some(cwd.join("target").join("debug").join("fake-sibling"))
        );

        // nothing anywhere -> None.
        assert_eq!(
            workspace_bin_in_dirs(Some(&exe_dir2), Some(&base.join("nowhere")), "fake-sibling"),
            None
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn resolve_tool_candidates_finds_fabricated_path() {
        // Serialize PATH mutation and reading against other tests.
        let _lock = crate::toolchain::ambient_path_lock()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let prev_path = std::env::var_os("PATH").unwrap_or_default();
        let base = temp_dir("path");
        let fake = write_script(&base, "fake-tool", "#!/bin/sh\nexit 0\n");
        std::env::set_var("PATH", &base);
        let got = resolve_tool_candidates(&["fake-tool"]);
        std::env::set_var("PATH", prev_path);
        assert_eq!(got.ok().as_deref(), Some(fake.as_path()));
        fs::remove_dir_all(&base).unwrap();
    }

    // --- probe dispatch (§6.2) --------------------------------------------
    #[test]
    fn probe_fasm_reads_stderr_with_no_args() {
        let base = temp_dir("fasm");
        // fasm prints to stderr; "fasm" must be probed with NO args.
        let fake = write_script(
            &base,
            "fasm",
            "#!/bin/sh\necho 'flat assembler  version 1.73.32  (16384 kilobytes memory)' >&2\n",
        );
        let v = probe_version(&fake);
        assert_eq!(v.as_deref(), Some("1.73.32"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn probe_rustc_first_token() {
        let base = temp_dir("rustc");
        let fake = write_script(
            &base,
            "rustc",
            "#!/bin/sh\necho 'rustc 1.101.0-nightly (c1070d69382b8d2f2eb65119c738a77d9e324c9e 2026-09-28)'\n",
        );
        let v = probe_version(&fake);
        assert_eq!(v.as_deref(), Some("1.101.0-nightly"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn probe_qemu_version_token() {
        let base = temp_dir("qemu");
        let fake = write_script(
            &base,
            "qemu-system-x86_64",
            "#!/bin/sh\necho 'QEMU emulator version 8.2.2 (Debian 1:8.2.2)'\n",
        );
        let v = probe_version(&fake);
        assert_eq!(v.as_deref(), Some("8.2.2"));
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn probe_default_first_line() {
        let base = temp_dir("ld");
        let fake = write_script(
            &base,
            "ld",
            "#!/bin/sh\necho 'GNU ld (GNU Binutils for Debian) 2.42.0'\n",
        );
        let v = probe_version(&fake);
        assert_eq!(
            v.as_deref(),
            Some("GNU ld (GNU Binutils for Debian) 2.42.0")
        );
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn probe_langc_is_presence_only() {
        let base = temp_dir("langc");
        // langc/tyu are never version-probed (§6.2): presence only.
        let fake = write_script(&base, "langc", "#!/bin/sh\necho 'langc 0.1.0'\n");
        assert_eq!(probe_version(&fake), None);
        fs::remove_dir_all(&base).unwrap();
    }

    // --- FR-22: toolchain check layout is byte-stable -----------------------
    #[test]
    fn toolchain_check_layout_is_byte_stable() {
        // The `toolchain check` report FORMAT is the contract (FR-22
        // consolidation: one resolution, two renderers — this layout must
        // not drift when doctor shares the resolution object). Paths are
        // environment data, so the assertion builds the expected string from
        // the same resolution doctor would produce.
        let _lock = crate::toolchain::ambient_path_lock()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let prev_path = std::env::var_os("PATH").unwrap_or_default();
        let base = temp_dir("tc-layout");
        let b = base.join("bin");
        write_script(
            &b,
            "fasm",
            "#!/bin/sh\necho 'flat assembler  version 1.73.32'\n",
        );
        write_script(&b, "ld", "#!/bin/sh\necho 'GNU ld 2.42'\n");
        write_script(&b, "nm", "#!/bin/sh\necho 'GNU nm 2.42'\n");
        write_script(
            &b,
            "qemu-system-x86_64",
            "#!/bin/sh\necho 'QEMU emulator version 8.2.2'\n",
        );
        std::env::set_var("PATH", &b);

        let manifest = ProjectManifest::default();
        let target = Target::X86_64UnknownNone;
        let res = resolve_tools(target, &manifest, &Default::default());
        let compiler = res.compiler.as_ref().expect("workspace langc resolves");
        let report = toolchain_check(target, &manifest);

        let expected = format!(
            "Toolchain for x86_64-unknown-none:\n\
             \x20 compiler: found@{} [workspace target dir]\n\
             \x20 assembler: found@{} [PATH] (1.73.32)\n\
             \x20 linker: found@{} [PATH] (GNU ld 2.42)\n\
             \x20 qemu: found@{} [PATH] (8.2.2)\n",
            compiler.path.display(),
            b.join("fasm").display(),
            b.join("ld").display(),
            b.join("qemu-system-x86_64").display(),
        );
        assert_eq!(
            report, expected,
            "toolchain check layout is byte-stable (FR-22)"
        );

        std::env::set_var("PATH", prev_path);
        fs::remove_dir_all(&base).unwrap();
    }

    // --- probe deadline ----------------------------------------------------
    #[test]
    fn probe_deadline_kills_a_hanging_tool() {
        let base = temp_dir("hang");
        let fake = write_script(&base, "hang", "#!/bin/sh\nsleep 60\n");
        let start = std::time::Instant::now();
        let out = run_probe(&fake, &["--version"], ProbeStream::Stdout);
        assert_eq!(out, None);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "deadline must kill the probe"
        );
        fs::remove_dir_all(&base).unwrap();
    }
}
