//! Minimal argument parser for the `tyu` toolchain driver.

use std::path::PathBuf;
use std::time::Duration;

use codegen_core::Target;

/// Top-level subcommands.
#[derive(Debug)]
pub enum Command {
    Build(BuildArgs),
    Run(RunArgs),
    Test(TestArgs),
    ToolchainCheck(ToolchainCheckArgs),
    Clean,
    Help,
}

/// Arguments for the `toolchain check` subcommand.
#[derive(Debug)]
pub struct ToolchainCheckArgs {
    /// Target triple or alias name.
    pub target: String,
}

/// Arguments for the `build` subcommand.
#[derive(Debug)]
pub struct BuildArgs {
    pub target: Target,
    pub input: PathBuf,
    pub include_dirs: Vec<PathBuf>,
    pub sysroot: Option<PathBuf>,
    pub out_dir: PathBuf,
}

/// Arguments for the `run` subcommand.
#[derive(Debug)]
pub struct RunArgs {
    pub target: Target,
    pub input: PathBuf,
    pub include_dirs: Vec<PathBuf>,
    pub sysroot: Option<PathBuf>,
    pub out_dir: PathBuf,
    pub timeout: Duration,
    pub runner_override: Option<String>,
}

impl RunArgs {
    pub fn to_build_args(&self) -> BuildArgs {
        BuildArgs {
            target: self.target,
            input: self.input.clone(),
            include_dirs: self.include_dirs.clone(),
            sysroot: self.sysroot.clone(),
            out_dir: self.out_dir.clone(),
        }
    }
}

/// Arguments for the `test` subcommand.
#[derive(Debug)]
pub struct TestArgs {
    pub target: Target,
    pub all_targets: bool,
    pub filter: Option<String>,
    pub manifest_path: PathBuf,
}

pub fn parse() -> Command {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        return Command::Help;
    }

    match args[1].as_str() {
        "build" => parse_build(&args[2..]),
        "run" => parse_run(&args[2..]),
        "test" => parse_test(&args[2..]),
        "toolchain" => parse_toolchain(&args[2..]),
        "clean" => Command::Clean,
        "--help" | "-h" => { print_usage(); Command::Help }
        other => {
            eprintln!("tyu: unknown command '{}'", other);
            Command::Help
        }
    }
}

fn print_usage() {
    eprintln!("usage: tyu <command> [options]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  build    Compile, assemble, and link an image");
    eprintln!("  run      Build and execute an image");
    eprintln!("  test     Discover and run test suites");
    eprintln!("  clean    Remove build artifacts");
    eprintln!();
    eprintln!("Build/Run options:");
    eprintln!("  --target=<triple>   Target triple");
    eprintln!("  --sysroot=<dir>     Sysroot directory");
    eprintln!("  --out-dir=<dir>     Output directory");
    eprintln!("  -I <dir>            Add include directory");
    eprintln!();
    eprintln!("Test options:");
    eprintln!("  --target=<triple>   Target triple (default: x86_64-unknown-linux-gnu)");
    eprintln!("  --all-targets       Run on all supported targets");
    eprintln!("  --filter=<pat>      Only run suites matching pattern");
    eprintln!("  --manifest=<path>   Path to manifest.toml");
    eprintln!();
    eprintln!("Toolchain options:");
    eprintln!("  tyu toolchain check <target>   Resolve and report tool paths");
    eprintln!();
    eprintln!("Run-specific options:");
    eprintln!("  --timeout=<secs>    Maximum execution time (default: 10)");
    eprintln!("  --runner=<mode>     Runner: native|qemu (default: auto)");
}

fn parse_common(args: &[String]) -> CommonArgs {
    let mut target: Option<Target> = None;
    let mut input: Option<PathBuf> = None;
    let mut include_dirs: Vec<PathBuf> = Vec::new();
    let mut sysroot: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(val) = a.strip_prefix("--target=") {
            let tb = val.as_bytes();
            target = Target::parse(tb);
            if target.is_none() {
                eprintln!("tyu: unknown target '{}'", val);
            }
        } else if let Some(val) = a.strip_prefix("--sysroot=") {
            sysroot = Some(PathBuf::from(val));
        } else if let Some(val) = a.strip_prefix("--out-dir=") {
            out_dir = Some(PathBuf::from(val));
        } else if a == "-I" {
            i += 1;
            if i < args.len() {
                include_dirs.push(PathBuf::from(&args[i]));
            } else {
                eprintln!("tyu: -I requires a value");
            }
        } else if a.starts_with('-') {
            // Skip.
        } else {
            input = Some(PathBuf::from(a));
        }
        i += 1;
    }

    let target = target.unwrap_or(Target::X86_64UnknownLinuxGnu);
    let out_dir = out_dir.unwrap_or_else(|| {
        let triple = std::str::from_utf8(target.triple()).unwrap();
        PathBuf::from("target").join("tyu").join(triple)
    });

    CommonArgs { target, input, include_dirs, sysroot, out_dir }
}

struct CommonArgs {
    target: Target,
    input: Option<PathBuf>,
    include_dirs: Vec<PathBuf>,
    sysroot: Option<PathBuf>,
    out_dir: PathBuf,
}

fn parse_build(args: &[String]) -> Command {
    let common = parse_common(args);
    let input = match common.input {
        Some(p) => p,
        None => { eprintln!("tyu: build requires an input .mod file"); return Command::Help; }
    };
    Command::Build(BuildArgs {
        target: common.target,
        input,
        include_dirs: common.include_dirs,
        sysroot: common.sysroot,
        out_dir: common.out_dir,
    })
}

fn parse_run(args: &[String]) -> Command {
    let common = parse_common(args);
    let mut timeout = Duration::from_secs(10);
    let mut runner_override: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(val) = a.strip_prefix("--timeout=") {
            let secs: u64 = match val.parse() {
                Ok(v) => v,
                Err(_) => { eprintln!("tyu: invalid --timeout"); return Command::Help; }
            };
            timeout = Duration::from_secs(secs);
        } else if let Some(val) = a.strip_prefix("--runner=") {
            runner_override = Some(val.to_string());
        }
        i += 1;
    }

    let input = match common.input {
        Some(p) => p,
        None => { eprintln!("tyu: run requires an input .mod file"); return Command::Help; }
    };

    Command::Run(RunArgs {
        target: common.target, input, include_dirs: common.include_dirs,
        sysroot: common.sysroot, out_dir: common.out_dir,
        timeout, runner_override,
    })
}

fn parse_test(args: &[String]) -> Command {
    let mut target: Option<Target> = None;
    let mut all_targets = false;
    let mut filter: Option<String> = None;
    let mut manifest_path: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(val) = a.strip_prefix("--target=") {
            let tb = val.as_bytes();
            target = Target::parse(tb);
            if target.is_none() {
                eprintln!("tyu: unknown target '{}'", val);
                return Command::Help;
            }
        } else if let Some(val) = a.strip_prefix("--filter=") {
            filter = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--manifest=") {
            manifest_path = Some(PathBuf::from(val));
        } else if a == "--all-targets" {
            all_targets = true;
        } else if a.starts_with('-') {
            eprintln!("tyu: unknown option '{}'", a);
            return Command::Help;
        }
        i += 1;
    }

    let target = target.unwrap_or(Target::X86_64UnknownLinuxGnu);
    let manifest_path = manifest_path.unwrap_or_else(default_manifest);

    Command::Test(TestArgs { target, all_targets, filter, manifest_path })
}

/// Default manifest path: `fixtures/manifest.toml` relative to CWD.
fn default_manifest() -> PathBuf {
    PathBuf::from("fixtures").join("manifest.toml")
}

fn parse_toolchain(args: &[String]) -> Command {
    if args.is_empty() || args[0] != "check" {
        eprintln!("tyu: usage: tyu toolchain check <target>");
        return Command::Help;
    }
    if args.len() < 2 {
        eprintln!("tyu: toolchain check requires a target triple or alias");
        return Command::Help;
    }
    Command::ToolchainCheck(ToolchainCheckArgs {
        target: args[1].clone(),
    })
}
