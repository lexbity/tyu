//! Pluggable backend runner abstraction.
//!
//! Executes a built image: natively (hosted target), under QEMU (bare-metal
//! target), or on a physical device (forward-looking).

use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use codegen_core::target::QemuSpec;
use codegen_core::Target;

/// How to run a built image.
pub enum Runner {
    /// Execute the ELF directly on the host (no QEMU).
    Native,
    /// Execute under QEMU system-mode using the given spec.
    Qemu(&'static QemuSpec),
    /// Physical hardware (stub — not yet implemented).
    Device,
}

/// Outcome of running an image.
#[derive(Debug)]
pub struct RunOutcome {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub timed_out: bool,
}

impl Runner {
    /// Select the default runner for a target.
    pub fn for_target(target: Target) -> Self {
        match target.spec().qemu {
            Some(spec) => Runner::Qemu(spec),
            None => Runner::Native,
        }
    }

    /// Run the image with the given timeout.
    ///
    /// Captures stdout.  Returns an error if the binary cannot be spawned
    /// (e.g. missing QEMU).  A timed-out process is killed and marked with
    /// `timed_out = true`.
    pub fn run(&self, image: &Path, timeout: Duration) -> Result<RunOutcome, String> {
        match self {
            Runner::Native => run_native(image, timeout),
            Runner::Qemu(spec) => run_qemu(spec, image, timeout),
            Runner::Device => Err("Device runner not yet implemented".into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Spawn a process and wait with timeout, capturing stdout.
fn spawn_and_wait(
    cmd: &mut Command,
    image: &Path,
    timeout: Duration,
) -> Result<RunOutcome, String> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = cmd.spawn()
        .map_err(|e| format!("spawning '{}': {}", image.display(), e))?;

    // Read stdout in a background thread so the pipe does not deadlock.
    let mut stdout_pipe = child.stdout.take()
        .ok_or("failed to capture stdout")?;
    let stdout_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout_pipe, &mut buf);
        buf
    });

    let start = Instant::now();
    let mut sleep_ms = 1u64;

    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break status.code().unwrap_or(-1);
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let out = stdout_handle.join().unwrap_or_default();
                    return Ok(RunOutcome {
                        exit_code: -1,
                        stdout: out,
                        timed_out: true,
                    });
                }
                thread::sleep(Duration::from_millis(sleep_ms));
                if sleep_ms < 50 {
                    sleep_ms += 1;
                }
            }
            Err(e) => {
                let _stdout = stdout_handle.join().unwrap_or_default();
                return Err(format!("waitpid failed: {}", e));
            }
        }
    };

    let stdout = stdout_handle.join().unwrap_or_default();
    Ok(RunOutcome {
        exit_code,
        stdout,
        timed_out: false,
    })
}

// ---------------------------------------------------------------------------
// Native runner
// ---------------------------------------------------------------------------

fn run_native(image: &Path, timeout: Duration) -> Result<RunOutcome, String> {
    let mut cmd = Command::new(image);
    spawn_and_wait(&mut cmd, image, timeout)
}

// ---------------------------------------------------------------------------
// QEMU runner
// ---------------------------------------------------------------------------

fn run_qemu(spec: &QemuSpec, image: &Path, timeout: Duration) -> Result<RunOutcome, String> {
    let bin = std::str::from_utf8(spec.system_bin)
        .map_err(|_| "non-UTF-8 QEMU binary name")?;
    let machine = std::str::from_utf8(spec.machine)
        .map_err(|_| "non-UTF-8 QEMU machine name")?;

    let mut cmd = Command::new(bin);
    cmd.arg("-machine").arg(machine);

    for arg in spec.extra_args {
        let s = std::str::from_utf8(arg)
            .map_err(|_| "non-UTF-8 QEMU extra arg")?;
        cmd.arg(s);
    }

    match spec.exit_convention {
        codegen_core::QemuExitConvention::Semihosting => {
            cmd.arg("-semihosting-config");
            cmd.arg("enable=on,target=native");
        }
        codegen_core::QemuExitConvention::IsaDebugExit { .. } => {}
    }

    cmd.arg("-kernel").arg(image);

    spawn_and_wait(&mut cmd, image, timeout)
}
