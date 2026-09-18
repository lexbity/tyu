//! `tyu` — toolchain driver for the tyu_lang language.

use std::path::Path;
use tyu::project::ProjectManifest;
use tyu::{args, build, deploy, platform, run_cmd, test_cmd, toolchain};

fn main() {
    let cwd = std::env::current_dir().unwrap_or_default();

    let project_manifest = tyu::project::find_manifest(&cwd)
        .and_then(|p| {
            eprintln!("tyu: using project manifest '{}'", p.display());
            tyu::project::parse_project_manifest(&p).ok()
        })
        .unwrap_or_default();

    match args::parse() {
        args::Command::Build(mut build_args) => {
            apply_project_to_build(&mut build_args, &project_manifest, &cwd);
            resolve_profile(
                &mut build_args.feature_set,
                build_args.profile.as_deref(),
                &project_manifest,
            );
            match build::build(&build_args) {
                Ok(image) => println!("{}", image.display()),
                Err(e) => {
                    eprintln!("tyu: build error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        args::Command::Run(mut run_args) => {
            apply_project_to_run(&mut run_args, &project_manifest, &cwd);
            resolve_profile(
                &mut run_args.feature_set,
                run_args.profile.as_deref(),
                &project_manifest,
            );
            if let Err(e) = run_cmd::run(&run_args) {
                eprintln!("tyu: run error: {}", e);
                std::process::exit(1);
            }
        }
        args::Command::Test(mut test_args) => {
            apply_project_to_test(&mut test_args, &project_manifest, &cwd);
            resolve_profile(
                &mut test_args.feature_set,
                test_args.profile.as_deref(),
                &project_manifest,
            );
            eprintln!("tyu: test features: [{}]", {
                let mut buf = [""; 8];
                let n = test_args.feature_set.write_flags(&mut buf);
                buf[..n].join(", ")
            });
            if let Err(e) = test_cmd::run(&test_args) {
                eprintln!("tyu: test error: {}", e);
                std::process::exit(1);
            }
        }
        args::Command::Deploy(deploy_args) => {
            if let Err(e) = deploy::run(&deploy_args) {
                eprintln!("tyu: deploy error: {}", e);
                std::process::exit(1);
            }
        }
        args::Command::Platform(platform_args) => {
            if let Err(e) = platform::run(platform_args) {
                eprintln!("tyu: platform error: {}", e);
                std::process::exit(1);
            }
        }
        args::Command::ToolchainCheck(tc_args) => {
            let target = tyu::project::resolve_target(&tc_args.target, &project_manifest)
                .or_else(|| codegen_core::Target::parse(tc_args.target.as_bytes()));

            match target {
                Some(t) => {
                    let report = toolchain::toolchain_check(t, &project_manifest);
                    print!("{}", report);
                }
                None => {
                    eprintln!("tyu: unknown target '{}'", tc_args.target);
                    std::process::exit(1);
                }
            }
        }
        args::Command::Clean => {
            let target_dir = std::path::Path::new("target").join("tyu");
            if target_dir.exists() {
                let _ = std::fs::remove_dir_all(&target_dir);
            }
        }
        args::Command::Help => {}
        args::Command::Usage => std::process::exit(2),
    }
}

/// Resolve a profile name to a `FeatureSet`, printing the result.
/// An unresolvable profile is a usage error (exit 2), not a silent
/// all-features-on fallback — a typo must not quietly change what builds.
fn resolve_profile(
    feature_set: &mut codegen_core::FeatureSet,
    profile_name: Option<&str>,
    manifest: &tyu::project::ProjectManifest,
) {
    match tyu::project::resolve_feature_set(profile_name, manifest) {
        Ok((set, name)) => {
            *feature_set = set;
            let mut flag_buf = [""; 8];
            let n = set.write_flags(&mut flag_buf);
            let profile_label = name.as_deref().unwrap_or("(implicit all-features-on)");
            eprintln!(
                "tyu: resolved profile '{}' → features: [{}]",
                profile_label,
                flag_buf[..n].join(", "),
            );
        }
        Err(e) => {
            eprintln!("tyu: profile resolution error: {}", e);
            std::process::exit(2);
        }
    }
}

fn apply_project_to_build(args: &mut args::BuildArgs, manifest: &ProjectManifest, cwd: &Path) {
    if let Some(input_str) = args.input.to_str() {
        let first = input_str.split('/').next().unwrap_or(input_str);
        if let Some(alias) = manifest.targets.get(first) {
            let remainder = input_str.trim_start_matches(first).trim_start_matches('/');
            let new_input = if remainder.is_empty() {
                args.input.clone()
            } else {
                cwd.join(remainder)
            };
            if let Some(t) = codegen_core::Target::parse(alias.triple.as_bytes()) {
                args.target = t;
                args.input = new_input;
            }
        }
    }
    if args.sysroot.is_none() {
        let candidate = cwd.join("sysroot");
        if candidate.is_dir() {
            args.sysroot = Some(candidate);
        }
    }
}

fn apply_project_to_run(args: &mut args::RunArgs, manifest: &ProjectManifest, cwd: &Path) {
    if let Some(input_str) = args.input.to_str() {
        let first = input_str.split('/').next().unwrap_or(input_str);
        if let Some(alias) = manifest.targets.get(first) {
            let remainder = input_str.trim_start_matches(first).trim_start_matches('/');
            let new_input = if remainder.is_empty() {
                args.input.clone()
            } else {
                cwd.join(remainder)
            };
            if let Some(t) = codegen_core::Target::parse(alias.triple.as_bytes()) {
                args.target = t;
                args.input = new_input;
            }
        }
    }
    if args.sysroot.is_none() {
        let candidate = cwd.join("sysroot");
        if candidate.is_dir() {
            args.sysroot = Some(candidate);
        }
    }
}

fn apply_project_to_test(_args: &mut args::TestArgs, _manifest: &ProjectManifest, _cwd: &Path) {}
