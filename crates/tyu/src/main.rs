//! `tyu` — toolchain driver for the tyu_lang language.

use std::path::Path;
use tyu::project::ProjectManifest;
use tyu::{args, deploy, run_cmd, test_cmd, toolchain, build};

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
            match build::build(&build_args) {
                Ok(image) => println!("{}", image.display()),
                Err(e) => { eprintln!("tyu: build error: {}", e); std::process::exit(1); }
            }
        }
        args::Command::Run(mut run_args) => {
            apply_project_to_run(&mut run_args, &project_manifest, &cwd);
            if let Err(e) = run_cmd::run(&run_args) {
                eprintln!("tyu: run error: {}", e);
                std::process::exit(1);
            }
        }
        args::Command::Test(mut test_args) => {
            apply_project_to_test(&mut test_args, &project_manifest, &cwd);
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
