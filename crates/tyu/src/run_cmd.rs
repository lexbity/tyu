//! `tyu run` subcommand — build and execute an image, classify the result.

use crate::args::RunArgs;
use crate::build;
use crate::runner::Runner;

/// Exit codes for failure classes (matches common convention):
const EXIT_HANG: i32 = 124;         // timeout
const EXIT_NO_COMPLETION: i32 = 1;  // S\n missing
const EXIT_FAIL_MARKER: i32 = 2;    // F byte seen
const EXIT_MISMATCH: i32 = 3;       // wrong QEMU exit code

/// Execute the `run` subcommand.
pub fn run(args: &RunArgs) -> Result<(), String> {
    // Build the image first.
    let build_args = args.to_build_args();
    let image = build::build(&build_args)?;

    // Determine the runner.
    let runner = if let Some(r) = &args.runner_override {
        match r.as_str() {
            "native" => Runner::Native,
            "qemu" => {
                let spec = args.target.spec().qemu
                    .ok_or("target has no QEMU spec — cannot use --runner=qemu")?;
                Runner::Qemu(spec)
            }
            other => return Err(format!("unknown runner '{}'", other)),

        }
    } else {
        Runner::for_target(args.target)
    };

    // Run with timeout.
    let outcome = runner.run(&image, args.timeout)?;

    // Parse output markers.
    let summary = harness_core::parse_output(&outcome.stdout);

    // Classify failures.
    if outcome.timed_out {
        eprintln!("tyu: HANG — image did not exit within {:?}", args.timeout);
        std::process::exit(EXIT_HANG);
    }

    if !summary.completed {
        eprintln!(
            "tyu: NO_COMPLETION — exited with code {} but no `S\\n` marker",
            outcome.exit_code,
        );
        std::process::exit(EXIT_NO_COMPLETION);
    }

    if summary.failures > 0 {
        eprintln!(
            "tyu: FAIL_MARKER — {} failure(s) reported",
            summary.failures,
        );
        std::process::exit(EXIT_FAIL_MARKER);
    }

    // For QEMU targets, check that the exit code matches the expected pass code.
    if args.target.spec().qemu.is_some() {
        let spec = args.target.spec().qemu.unwrap();
        let expected = spec.exit_convention.host_pass_exit();
        if outcome.exit_code != expected {
            eprintln!(
                "tyu: EXIT_MISMATCH — exit code {} != expected {}",
                outcome.exit_code, expected,
            );
            std::process::exit(EXIT_MISMATCH);
        }
    } else {
        // Native target: expect exit code 0.
        if outcome.exit_code != 0 {
            eprintln!(
                "tyu: EXIT_MISMATCH — native exit code {} != 0",
                outcome.exit_code,
            );
            std::process::exit(EXIT_MISMATCH);
        }
    }

    Ok(())
}
