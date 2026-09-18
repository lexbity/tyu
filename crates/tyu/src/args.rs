//! Minimal argument parser for the `tyu` toolchain driver.

use std::path::PathBuf;
use std::time::Duration;

use codegen_core::{FeatureSet, Target};

use crate::platform;

/// Top-level subcommands.
#[derive(Debug)]
pub enum Command {
    Build(BuildArgs),
    Run(RunArgs),
    Test(TestArgs),
    Deploy(DeployArgs),
    Platform(PlatformArgs),
    ToolchainCheck(ToolchainCheckArgs),
    Clean,
    /// Help was explicitly requested (`--help`/`-h`); exit 0.
    Help,
    /// Argument parsing failed (unknown flag/command, bad value); the error
    /// has already been printed to stderr. Dispatched to a non-zero exit so
    /// scripted invocations don't silently "pass" on a typo'd flag.
    Usage,
}

/// Arguments for the `platform` subcommand.
#[derive(Debug)]
pub enum PlatformArgs {
    List,
    Info { name: String, isa: Option<String> },
    Lint { name: String, all: bool },
    New { name: String },
}

/// Arguments for the `toolchain check` subcommand.
#[derive(Debug)]
pub struct ToolchainCheckArgs {
    pub target: String,
}

/// Encryption mode for deploy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EncryptMode {
    None,
    Fleet,
    Device,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BuildMode {
    Static,
    Dynamic,
}

impl BuildMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "static" => Some(Self::Static),
            "dynamic" => Some(Self::Dynamic),
            _ => None,
        }
    }
}

/// Arguments for the `deploy` subcommand.
#[derive(Debug)]
pub struct DeployArgs {
    pub target: Target,
    pub platform: Option<String>,
    pub isa: Option<String>,
    pub input: PathBuf,
    pub include_dirs: Vec<PathBuf>,
    pub sysroot: Option<PathBuf>,
    pub out_dir: PathBuf,
    pub enc_mode: EncryptMode,
    pub key_encrypt: Option<String>,
    pub key_sign: Option<String>,
    pub sign: bool,
    pub commit_otp: bool,
    pub device_keys_dir: Option<PathBuf>,
    pub profile: Option<String>,
    pub feature_set: FeatureSet,
}

impl DeployArgs {
    pub fn to_build_args(&self) -> BuildArgs {
        BuildArgs {
            target: self.target,
            platform: self.platform.clone(),
            isa: self.isa.clone(),
            input: self.input.clone(),
            include_dirs: self.include_dirs.clone(),
            sysroot: self.sysroot.clone(),
            out_dir: self.out_dir.clone(),
            profile: self.profile.clone(),
            feature_set: self.feature_set,
            mode: None,
            metal_sign_key: None,
            metal_kek: None,
            metal_encrypt_mode: None,
        }
    }
}

/// Arguments for the `build` subcommand.
#[derive(Debug)]
pub struct BuildArgs {
    pub target: Target,
    pub platform: Option<String>,
    pub isa: Option<String>,
    pub input: PathBuf,
    pub include_dirs: Vec<PathBuf>,
    pub sysroot: Option<PathBuf>,
    pub out_dir: PathBuf,
    /// Profile name from `--profile=<name>`, resolved to `feature_set` in main.rs.
    pub profile: Option<String>,
    /// Resolved feature set (set by main.rs after profile resolution).
    pub feature_set: FeatureSet,
    pub mode: Option<BuildMode>,
    pub metal_sign_key: Option<String>,
    pub metal_kek: Option<String>,
    pub metal_encrypt_mode: Option<EncryptMode>,
}

/// Arguments for the `run` subcommand.
#[derive(Debug)]
pub struct RunArgs {
    pub target: Target,
    pub platform: Option<String>,
    pub isa: Option<String>,
    pub input: PathBuf,
    pub include_dirs: Vec<PathBuf>,
    pub sysroot: Option<PathBuf>,
    pub out_dir: PathBuf,
    pub profile: Option<String>,
    pub feature_set: FeatureSet,
    pub timeout: Duration,
    pub runner_override: Option<String>,
    pub mode: Option<BuildMode>,
    pub metal_sign_key: Option<String>,
    pub metal_kek: Option<String>,
    pub metal_encrypt_mode: Option<EncryptMode>,
}

impl RunArgs {
    pub fn to_build_args(&self) -> BuildArgs {
        BuildArgs {
            target: self.target,
            platform: self.platform.clone(),
            isa: self.isa.clone(),
            input: self.input.clone(),
            include_dirs: self.include_dirs.clone(),
            sysroot: self.sysroot.clone(),
            out_dir: self.out_dir.clone(),
            profile: self.profile.clone(),
            feature_set: self.feature_set,
            mode: self.mode,
            metal_sign_key: self.metal_sign_key.clone(),
            metal_kek: self.metal_kek.clone(),
            metal_encrypt_mode: self.metal_encrypt_mode,
        }
    }
}

/// Arguments for the `test` subcommand.
#[derive(Debug)]
pub struct TestArgs {
    pub target: Target,
    pub platform: Option<String>,
    pub isa: Option<String>,
    pub all_targets: bool,
    pub all_platforms: bool,
    pub filter: Option<String>,
    pub manifest_path: PathBuf,
    pub profile: Option<String>,
    pub feature_set: FeatureSet,
    pub qualify: bool,
    pub format: ReportFormat,
    pub report_out: Option<PathBuf>,
    /// Link/load mode for built fixture images. `None`/`Static` uses the
    /// fixture+runner static-link path; `Dynamic` is gated (see `test_cmd::run`).
    pub mode: Option<BuildMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Human,
    Json,
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
        "deploy" => parse_deploy(&args[2..]),
        "platform" => parse_platform(&args[2..]),
        "toolchain" => parse_toolchain(&args[2..]),
        "clean" => Command::Clean,
        "--help" | "-h" => {
            print_usage();
            Command::Help
        }
        other => {
            eprintln!("tyu: unknown command '{}'", other);
            Command::Usage
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
    eprintln!("  platform Inspect discovered platform packs");
    eprintln!("  clean    Remove build artifacts");
    eprintln!();
    eprintln!("Build/Run options:");
    eprintln!("  --target=<triple>   Target triple");
    eprintln!("  --platform=<name>   Platform pack name");
    eprintln!("  --isa=<arch>        ISA filter for platform packs");
    eprintln!("  --profile=<name>    Build profile from tyu.toml [profile.<name>]");
    eprintln!("  --sysroot=<dir>     Sysroot directory");
    eprintln!("  --out-dir=<dir>     Output directory");
    eprintln!("  -I <dir>            Add include directory");
    eprintln!();
    eprintln!("Test options:");
    eprintln!("  --target=<triple>   Target triple (default: x86_64-unknown-linux-gnu)");
    eprintln!("  --platform=<name>   Platform pack name");
    eprintln!("  --isa=<arch>        ISA filter for platform packs");
    eprintln!("  --all-targets       Run on all supported targets");
    eprintln!("  --all-platforms     Run all discovered QEMU-capable platform packs");
    eprintln!("  --filter=<pat>      Only run suites matching pattern");
    eprintln!("  --manifest=<path>   Path to manifest.toml");
    eprintln!(
        "  --mode=<mode>       Link/load mode: static|dynamic (default: dynamic for QEMU targets)"
    );
    eprintln!("  --qualify           Fail when required coverage axes are uncovered");
    eprintln!("  --format=<mode>     Report format: human|json (default: human)");
    eprintln!("  --report-out=<path> Write structured report JSON to path");
    eprintln!();
    eprintln!("Platform options:");
    eprintln!("  tyu platform list                    List discovered packs");
    eprintln!("  tyu platform info <name> [--isa=A]   Show pack details");
    eprintln!("  tyu platform lint <name> [--all]     Validate a pack");
    eprintln!("  tyu platform new <name>              Scaffold a pack");
    eprintln!();
    eprintln!("Deploy options:");
    eprintln!("  --commit-otp        Request the guarded OTP commit path");
    eprintln!();
    eprintln!("Toolchain options:");
    eprintln!("  tyu toolchain check <target>   Resolve and report tool paths");
    eprintln!();
    eprintln!("Run-specific options:");
    eprintln!("  --timeout=<secs>    Maximum execution time (default: 10)");
    eprintln!("  --runner=<mode>     Runner: native|qemu (default: auto)");
    eprintln!("  --mode=<mode>       Link/load mode: static|dynamic");
    eprintln!("  --metal-sign-key=<keyref> Provision Tier-1 firmware HMAC key");
    eprintln!("  --metal-kek=<keyref>      Encrypt dynamic .lmod with a firmware KEK");
    eprintln!("  --metal-encrypt=<mode>    Metal encryption mode: fleet|device (default: fleet)");
}

fn parse_common(args: &[String], extra_known: &[&str]) -> Result<CommonArgs, ()> {
    let mut target: Option<Target> = None;
    let mut platform: Option<String> = None;
    let mut isa: Option<String> = None;
    let mut input: Option<PathBuf> = None;
    let mut include_dirs: Vec<PathBuf> = Vec::new();
    let mut sysroot: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut profile: Option<String> = None;
    let mut mode: Option<BuildMode> = None;
    let mut metal_sign_key: Option<String> = None;
    let mut metal_kek: Option<String> = None;
    let mut metal_encrypt_mode: Option<EncryptMode> = None;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(val) = a.strip_prefix("--target=") {
            let tb = val.as_bytes();
            target = Target::parse(tb);
            if target.is_none() {
                eprintln!("tyu: unknown target '{}'", val);
                return Err(());
            }
        } else if a == "--platform" {
            i += 1;
            if i < args.len() {
                platform = Some(args[i].clone());
            } else {
                eprintln!("tyu: --platform requires a value");
                return Err(());
            }
        } else if a == "--isa" {
            i += 1;
            if i < args.len() {
                isa = Some(args[i].clone());
            } else {
                eprintln!("tyu: --isa requires a value");
                return Err(());
            }
        } else if let Some(val) = a.strip_prefix("--platform=") {
            platform = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--isa=") {
            isa = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--profile=") {
            profile = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--mode=") {
            mode = BuildMode::parse(val);
            if mode.is_none() {
                eprintln!("tyu: invalid --mode '{}'", val);
                return Err(());
            }
        } else if let Some(val) = a.strip_prefix("--metal-sign-key=") {
            metal_sign_key = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--metal-kek=") {
            metal_kek = Some(val.to_string());
        } else if let Some(val) = a
            .strip_prefix("--metal-encrypt=")
            .or_else(|| a.strip_prefix("--metal-enc="))
        {
            metal_encrypt_mode = match val {
                "fleet" => Some(EncryptMode::Fleet),
                "device" => Some(EncryptMode::Device),
                _ => {
                    eprintln!("tyu: invalid --metal-encrypt '{}'", val);
                    return Err(());
                }
            };
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
                return Err(());
            }
        } else if a.starts_with('-') {
            // Unknown flags are an error on every subcommand (BUG-009).  The
            // only exceptions are flags the caller's own loop handles.
            if !flag_known(a, extra_known) {
                eprintln!("tyu: unknown option '{}'", a);
                return Err(());
            }
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

    Ok(CommonArgs {
        target,
        platform,
        isa,
        input,
        include_dirs,
        sysroot,
        out_dir,
        profile,
        mode,
        metal_sign_key,
        metal_kek,
        metal_encrypt_mode,
    })
}

/// True when the flag `a` is one the caller's own loop handles: an exact
/// match, or a prefix match for value-taking flags (listed with a trailing
/// `=`).
fn flag_known(a: &str, known: &[&str]) -> bool {
    known.iter().any(|k| {
        if a == *k {
            return true;
        }
        k.ends_with('=') && a.starts_with(k)
    })
}

struct CommonArgs {
    target: Target,
    platform: Option<String>,
    isa: Option<String>,
    input: Option<PathBuf>,
    include_dirs: Vec<PathBuf>,
    sysroot: Option<PathBuf>,
    out_dir: PathBuf,
    profile: Option<String>,
    mode: Option<BuildMode>,
    metal_sign_key: Option<String>,
    metal_kek: Option<String>,
    metal_encrypt_mode: Option<EncryptMode>,
}

fn parse_build(args: &[String]) -> Command {
    let common = match parse_common(args, &[]) {
        Ok(c) => c,
        Err(()) => return Command::Usage,
    };
    let input = match common.input {
        Some(p) => p,
        None => {
            eprintln!("tyu: build requires an input .mod file");
            return Command::Usage;
        }
    };
    Command::Build(BuildArgs {
        target: common.target,
        platform: common.platform,
        isa: common.isa,
        input,
        include_dirs: common.include_dirs,
        sysroot: common.sysroot,
        out_dir: common.out_dir,
        profile: common.profile,
        feature_set: FeatureSet::default(), // resolved in main.rs
        mode: common.mode,
        metal_sign_key: common.metal_sign_key,
        metal_kek: common.metal_kek,
        metal_encrypt_mode: common.metal_encrypt_mode,
    })
}

fn parse_run(args: &[String]) -> Command {
    let common = match parse_common(args, &["--timeout=", "--runner="]) {
        Ok(c) => c,
        Err(()) => return Command::Usage,
    };
    let mut timeout = Duration::from_secs(10);
    let mut runner_override: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(val) = a.strip_prefix("--timeout=") {
            let secs: u64 = match val.parse() {
                Ok(v) => v,
                Err(_) => {
                    eprintln!("tyu: invalid --timeout");
                    return Command::Usage;
                }
            };
            timeout = Duration::from_secs(secs);
        } else if let Some(val) = a.strip_prefix("--runner=") {
            runner_override = Some(val.to_string());
        }
        i += 1;
    }

    let input = match common.input {
        Some(p) => p,
        None => {
            eprintln!("tyu: run requires an input .mod file");
            return Command::Usage;
        }
    };

    Command::Run(RunArgs {
        target: common.target,
        platform: common.platform,
        isa: common.isa,
        input,
        include_dirs: common.include_dirs,
        sysroot: common.sysroot,
        out_dir: common.out_dir,
        profile: common.profile,
        feature_set: FeatureSet::default(),
        timeout,
        runner_override,
        mode: common.mode,
        metal_sign_key: common.metal_sign_key,
        metal_kek: common.metal_kek,
        metal_encrypt_mode: common.metal_encrypt_mode,
    })
}

fn parse_test(args: &[String]) -> Command {
    let mut target: Option<Target> = None;
    let mut target_explicit = false;
    let mut platform: Option<String> = None;
    let mut isa: Option<String> = None;
    let mut all_targets = false;
    let mut all_platforms = false;
    let mut filter: Option<String> = None;
    let mut manifest_path: Option<PathBuf> = None;
    let mut profile: Option<String> = None;
    let mut features: Option<FeatureSet> = None;
    let mut qualify = false;
    let mut format = ReportFormat::Human;
    let mut report_out: Option<PathBuf> = None;
    let mut mode: Option<BuildMode> = None;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(val) = a.strip_prefix("--target=") {
            target_explicit = true;
            let tb = val.as_bytes();
            target = Target::parse(tb);
            if target.is_none() {
                eprintln!("tyu: unknown target '{}'", val);
                return Command::Usage;
            }
        } else if let Some(val) = a.strip_prefix("--platform=") {
            platform = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--isa=") {
            isa = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--mode=") {
            mode = BuildMode::parse(val);
            if mode.is_none() {
                eprintln!("tyu: invalid --mode '{}' (expected static|dynamic)", val);
                return Command::Usage;
            }
        } else if let Some(val) = a.strip_prefix("--profile=") {
            profile = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--features=") {
            let mut set = FeatureSet::empty();
            for chunk in val.split(',') {
                let f = codegen_core::Feature::parse(chunk).unwrap_or_else(|| {
                    eprintln!("tyu: unknown feature '{}'", chunk);
                    std::process::exit(1);
                });
                set = set.with(f);
            }
            features = Some(set);
        } else if let Some(val) = a.strip_prefix("--profile=") {
            profile = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--filter=") {
            filter = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--manifest=") {
            manifest_path = Some(PathBuf::from(val));
        } else if let Some(val) = a.strip_prefix("--format=") {
            format = match val {
                "human" => ReportFormat::Human,
                "json" => ReportFormat::Json,
                _ => {
                    eprintln!("tyu: unknown test report format '{}'", val);
                    return Command::Usage;
                }
            };
        } else if let Some(val) = a.strip_prefix("--report-out=") {
            report_out = Some(PathBuf::from(val));
        } else if a == "--qualify" {
            qualify = true;
        } else if a == "--all-targets" {
            all_targets = true;
        } else if a == "--all-platforms" {
            all_platforms = true;
        } else if a.starts_with('-') {
            eprintln!("tyu: unknown option '{}'", a);
            return Command::Usage;
        }
        i += 1;
    }

    if all_targets && all_platforms {
        eprintln!("tyu: --all-targets and --all-platforms are mutually exclusive");
        return Command::Usage;
    }

    if all_targets && (platform.is_some() || isa.is_some()) {
        eprintln!("tyu: --all-targets cannot be combined with --platform/--isa");
        return Command::Usage;
    }

    if all_platforms && (platform.is_some() || isa.is_some() || target_explicit) {
        eprintln!("tyu: --all-platforms cannot be combined with --target/--platform/--isa");
        return Command::Usage;
    }

    let mut target = target.unwrap_or(Target::X86_64UnknownLinuxGnu);
    if let Some(ref platform_name) = platform {
        let selection = match platform::resolve_platform_selection(
            &platform::workspace_root(),
            platform_name,
            isa.as_deref(),
        ) {
            Ok(selection) => selection,
            Err(e) => {
                eprintln!("tyu: {}", e);
                return Command::Usage;
            }
        };
        if target_explicit && selection.target != target {
            eprintln!(
                "tyu: --target {} and --platform {} resolve to different targets",
                std::str::from_utf8(target.triple()).unwrap_or("<invalid>"),
                platform_name,
            );
            return Command::Usage;
        }
        target = selection.target;
    }

    let manifest_path = manifest_path.unwrap_or_else(default_manifest);

    Command::Test(TestArgs {
        target,
        platform,
        isa,
        all_targets,
        all_platforms,
        filter,
        manifest_path,
        profile,
        feature_set: features.unwrap_or(FeatureSet::all()),
        qualify,
        format,
        report_out,
        mode,
    })
}

/// Default manifest path: `fixtures/manifest.toml` relative to CWD.
fn default_manifest() -> PathBuf {
    PathBuf::from("fixtures").join("manifest.toml")
}

fn parse_deploy(args: &[String]) -> Command {
    let common = match parse_common(
        args,
        &[
            "--encrypt=",
            "--key-encrypt=",
            "--key-sign=",
            "--device-keys=",
            "--sign",
            "--commit-otp",
        ],
    ) {
        Ok(c) => c,
        Err(()) => return Command::Usage,
    };
    let mut enc_mode = EncryptMode::None;
    let mut key_encrypt: Option<String> = None;
    let mut key_sign: Option<String> = None;
    let mut sign = false;
    let mut commit_otp = false;
    let mut device_keys_dir: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(val) = a.strip_prefix("--encrypt=") {
            enc_mode = match val {
                "none" => EncryptMode::None,
                "fleet" => EncryptMode::Fleet,
                "device" => EncryptMode::Device,
                _ => {
                    eprintln!("tyu: unknown encrypt mode '{}'", val);
                    return Command::Usage;
                }
            };
        } else if let Some(val) = a.strip_prefix("--key-encrypt=") {
            key_encrypt = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--key-sign=") {
            key_sign = Some(val.to_string());
        } else if let Some(val) = a.strip_prefix("--device-keys=") {
            device_keys_dir = Some(PathBuf::from(val));
        } else if a == "--sign" {
            sign = true;
        } else if a == "--commit-otp" {
            commit_otp = true;
        }
        i += 1;
    }

    let input = match common.input {
        Some(p) => p,
        None => {
            eprintln!("tyu: deploy requires an input .mod file");
            return Command::Usage;
        }
    };

    Command::Deploy(DeployArgs {
        target: common.target,
        platform: common.platform,
        isa: common.isa,
        input,
        include_dirs: common.include_dirs,
        sysroot: common.sysroot,
        out_dir: common.out_dir,
        enc_mode,
        key_encrypt,
        key_sign,
        sign,
        commit_otp,
        device_keys_dir,
        profile: common.profile,
        feature_set: FeatureSet::default(),
    })
}

fn parse_platform(args: &[String]) -> Command {
    if args.is_empty() {
        eprintln!("tyu: platform requires a subcommand");
        print_usage();
        return Command::Usage;
    }

    match args[0].as_str() {
        "list" => Command::Platform(PlatformArgs::List),
        "info" => {
            let mut name: Option<String> = None;
            let mut isa: Option<String> = None;
            let mut i = 1;
            while i < args.len() {
                let a = &args[i];
                if let Some(val) = a.strip_prefix("--isa=") {
                    isa = Some(val.to_string());
                } else if a == "--isa" {
                    i += 1;
                    if i < args.len() {
                        isa = Some(args[i].clone());
                    } else {
                        eprintln!("tyu: --isa requires a value");
                        return Command::Usage;
                    }
                } else if a.starts_with('-') {
                    eprintln!("tyu: unknown option '{}'", a);
                    return Command::Usage;
                } else if name.is_none() {
                    name = Some(a.clone());
                }
                i += 1;
            }

            match name {
                Some(name) => Command::Platform(PlatformArgs::Info { name, isa }),
                None => {
                    eprintln!("tyu: platform info requires a pack name");
                    Command::Usage
                }
            }
        }
        "lint" => {
            let mut name: Option<String> = None;
            let mut all = false;
            let mut i = 1;
            while i < args.len() {
                let a = &args[i];
                if a == "--all" {
                    all = true;
                } else if a.starts_with('-') {
                    eprintln!("tyu: unknown option '{}'", a);
                    return Command::Usage;
                } else if name.is_none() {
                    name = Some(a.clone());
                }
                i += 1;
            }

            match name {
                Some(name) => Command::Platform(PlatformArgs::Lint { name, all }),
                None => {
                    eprintln!("tyu: platform lint requires a pack name");
                    Command::Usage
                }
            }
        }
        "new" => {
            if args.len() != 2 {
                eprintln!("tyu: platform new requires a pack name");
                return Command::Usage;
            }
            let name = args[1].clone();
            Command::Platform(PlatformArgs::New { name })
        }
        other => {
            eprintln!("tyu: unknown platform subcommand '{}'", other);
            Command::Usage
        }
    }
}

fn parse_toolchain(args: &[String]) -> Command {
    if args.is_empty() || args[0] != "check" {
        eprintln!("tyu: usage: tyu toolchain check <target>");
        return Command::Usage;
    }
    if args.len() < 2 {
        eprintln!("tyu: toolchain check requires a target triple or alias");
        return Command::Usage;
    }
    Command::ToolchainCheck(ToolchainCheckArgs {
        target: args[1].clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_platform_new() {
        match parse_platform(&strings(&["new", "demo"])) {
            Command::Platform(PlatformArgs::New { name }) => assert_eq!(name, "demo"),
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn parses_deploy_commit_otp_flag() {
        match parse_deploy(&strings(&["--commit-otp", "module.mod"])) {
            Command::Deploy(args) => assert!(args.commit_otp),
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn parses_test_mode_static_and_dynamic() {
        match parse_test(&strings(&["--mode=static"])) {
            Command::Test(args) => assert_eq!(args.mode, Some(BuildMode::Static)),
            other => panic!("unexpected command: {:?}", other),
        }
        match parse_test(&strings(&["--mode=dynamic"])) {
            Command::Test(args) => assert_eq!(args.mode, Some(BuildMode::Dynamic)),
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn test_mode_defaults_to_none() {
        match parse_test(&strings(&["--target=x86_64-unknown-none"])) {
            Command::Test(args) => assert_eq!(args.mode, None),
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn invalid_test_mode_is_usage_error() {
        assert!(matches!(
            parse_test(&strings(&["--mode=sideways"])),
            Command::Usage
        ));
    }

    #[test]
    fn unknown_test_flag_is_usage_error() {
        assert!(matches!(
            parse_test(&strings(&["--definitely-not-a-flag"])),
            Command::Usage
        ));
    }

    // -----------------------------------------------------------------------
    // BUG-009: unknown flags are an error on every subcommand, not skipped.
    // -----------------------------------------------------------------------

    #[test]
    fn unknown_build_flag_is_usage_error() {
        assert!(matches!(
            parse_build(&strings(&["--check=all", "Main.mod"])),
            Command::Usage
        ));
        assert!(matches!(
            parse_build(&strings(&["--definitely-not-a-flag", "Main.mod"])),
            Command::Usage
        ));
    }

    #[test]
    fn unknown_run_flag_is_usage_error() {
        assert!(matches!(
            parse_run(&strings(&["--check=all", "Main.tyu"])),
            Command::Usage
        ));
    }

    #[test]
    fn unknown_deploy_flag_is_usage_error() {
        assert!(matches!(
            parse_deploy(&strings(&["--definitely-not-a-flag", "Main.mod"])),
            Command::Usage
        ));
    }

    #[test]
    fn unknown_platform_info_flag_is_usage_error() {
        assert!(matches!(
            parse_platform(&strings(&["info", "demo", "--definitely-not-a-flag"])),
            Command::Usage
        ));
    }

    // -----------------------------------------------------------------------
    // BUG-013: invalid flag VALUES are an error, not warn-and-continue — a
    // typo'd --target must not silently build for the default target.
    // -----------------------------------------------------------------------

    #[test]
    fn unknown_target_value_is_usage_error() {
        assert!(matches!(
            parse_build(&strings(&["--target=x86_64-unknown-none-typo", "Main.mod"])),
            Command::Usage
        ));
        assert!(matches!(
            parse_run(&strings(&["--target=not-a-target", "Main.tyu"])),
            Command::Usage
        ));
    }

    #[test]
    fn invalid_mode_value_is_usage_error() {
        assert!(matches!(
            parse_build(&strings(&["--mode=stats", "Main.mod"])),
            Command::Usage
        ));
    }

    #[test]
    fn invalid_metal_encrypt_value_is_usage_error() {
        assert!(matches!(
            parse_build(&strings(&["--metal-encrypt=devise", "Main.mod"])),
            Command::Usage
        ));
    }

    #[test]
    fn trailing_valueless_flag_is_usage_error() {
        assert!(matches!(
            parse_build(&strings(&["Main.mod", "--platform"])),
            Command::Usage
        ));
        assert!(matches!(parse_build(&strings(&["Main.mod", "--isa"])), Command::Usage));
        assert!(matches!(parse_build(&strings(&["Main.mod", "-I"])), Command::Usage));
    }

    #[test]
    fn valid_flag_values_still_parse() {
        match parse_build(&strings(&[
            "--target=x86_64-unknown-none",
            "--mode=static",
            "--metal-encrypt=device",
            "Main.mod",
        ])) {
            Command::Build(args) => {
                assert_eq!(args.target, Target::X86_64UnknownNone);
                assert_eq!(args.mode, Some(BuildMode::Static));
                assert_eq!(args.metal_encrypt_mode, Some(EncryptMode::Device));
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn run_specific_flags_still_parse() {
        match parse_run(&strings(&["--timeout=5", "--runner=qemu", "Main.tyu"])) {
            Command::Run(args) => {
                assert_eq!(args.timeout, Duration::from_secs(5));
                assert_eq!(args.runner_override.as_deref(), Some("qemu"));
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn deploy_specific_flags_still_parse() {
        match parse_deploy(&strings(&[
            "--sign",
            "--commit-otp",
            "--encrypt=fleet",
            "--key-sign=k1",
            "Main.mod",
        ])) {
            Command::Deploy(args) => {
                assert!(args.sign);
                assert!(args.commit_otp);
                assert_eq!(args.enc_mode, EncryptMode::Fleet);
                assert_eq!(args.key_sign.as_deref(), Some("k1"));
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn parses_test_platform_selection() {
        match parse_test(&strings(&[
            "--platform=rp2350",
            "--isa=arm",
            "--manifest=fixtures/manifest.toml",
        ])) {
            Command::Test(args) => {
                assert!(args.platform.as_deref() == Some("rp2350"));
                assert!(args.isa.as_deref() == Some("arm"));
                assert_eq!(args.target, codegen_core::Target::ArmV7MUnknownNone);
                assert!(!args.all_platforms);
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn parses_test_all_platforms() {
        match parse_test(&strings(&[
            "--all-platforms",
            "--manifest=fixtures/manifest.toml",
        ])) {
            Command::Test(args) => {
                assert!(args.all_platforms);
                assert!(args.platform.is_none());
                assert!(args.isa.is_none());
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn parses_test_qualify_and_report_options() {
        match parse_test(&strings(&[
            "--qualify",
            "--format=json",
            "--report-out=/tmp/tyu-report.json",
            "--manifest=fixtures/manifest.toml",
        ])) {
            Command::Test(args) => {
                assert!(args.qualify);
                assert_eq!(args.format, ReportFormat::Json);
                assert_eq!(
                    args.report_out.as_deref(),
                    Some(Path::new("/tmp/tyu-report.json"))
                );
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }
}
