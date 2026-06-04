//! `tyu` — toolchain driver for the tyu_lang language.

mod args;
mod build;
mod cache;
mod graph;
mod highwater;
mod manifest;
mod project;
mod run_cmd;
mod runner;
mod test_cmd;
mod toml_parser;
mod toolchain;

use args::Command;

fn main() {
    match args::parse() {
        Command::Build(build_args) => {
            match build::build(&build_args) {
                Ok(image) => println!("{}", image.display()),
                Err(e) => { eprintln!("tyu: build error: {}", e); std::process::exit(1); }
            }
        }
        Command::Run(run_args) => {
            if let Err(e) = run_cmd::run(&run_args) {
                eprintln!("tyu: run error: {}", e);
                std::process::exit(1);
            }
        }
        Command::Test(test_args) => {
            if let Err(e) = test_cmd::run(&test_args) {
                eprintln!("tyu: test error: {}", e);
                std::process::exit(1);
            }
        }
        Command::ToolchainCheck(tc_args) => {
            let cwd = std::env::current_dir().unwrap_or_default();
            let manifest = project::find_manifest(&cwd)
                .and_then(|p| project::parse_project_manifest(&p).ok())
                .unwrap_or_default();

            // Try the argument as an alias first, then as a direct triple.
            let target = project::resolve_target(&tc_args.target, &manifest)
                .or_else(|| codegen_core::Target::parse(tc_args.target.as_bytes()));

            match target {
                Some(t) => {
                    let report = toolchain::toolchain_check(t, &manifest);
                    print!("{}", report);
                }
                None => {
                    eprintln!("tyu: unknown target '{}'", tc_args.target);
                    std::process::exit(1);
                }
            }
        }
        Command::Clean => {
            let target_dir = std::path::Path::new("target").join("tyu");
            if target_dir.exists() {
                let _ = std::fs::remove_dir_all(&target_dir);
            }
        }
        Command::Help => {}
    }
}
