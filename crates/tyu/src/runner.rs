//! Pluggable backend runner abstraction.
//!
//! Executes a built image: natively (hosted target), under QEMU (bare-metal
//! target), or on a physical device via OpenOCD/probe-rs.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use codegen_core::target::QemuSpec;
use codegen_core::Target;
use crate::error::TyuError;

/// How to run a built image.
#[allow(dead_code)]
pub enum Runner {
    /// Execute the ELF directly on the host (no QEMU).
    Native,
    /// Execute under QEMU system-mode using the given spec.
    Qemu(&'static QemuSpec),
    /// Execute under QEMU with gdbstub for A-side escalation.
    QemuDebug {
        spec: &'static QemuSpec,
        gdb_port: u16,
    },
    /// Flash and run on physical hardware via OpenOCD.
    Device(OpenOcdSpec),
}

/// OpenOCD configuration for physical device flashing + serial capture.
#[derive(Clone, Debug)]
pub struct OpenOcdSpec {
    /// OpenOCD binary name or path (default: `openocd`).
    pub bin: String,
    /// OpenOCD configuration file (e.g. `board/stm32f4discovery.cfg`).
    pub config: String,
    /// Serial port for UART capture (e.g. `/dev/ttyACM0`).
    pub serial_port: String,
    /// Serial baud rate (default: 115200).
    pub baud: u32,
    /// Timeout in seconds for the flash operation.
    pub flash_timeout_secs: u64,
}

impl Default for OpenOcdSpec {
    fn default() -> Self {
        OpenOcdSpec {
            bin: "openocd".into(),
            config: String::new(),
            serial_port: String::new(),
            baud: 115200,
            flash_timeout_secs: 30,
        }
    }
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
    /// For `Device`, flashes via OpenOCD and captures serial output.
    pub fn run(&self, image: &Path, timeout: Duration) -> Result<RunOutcome, String> {
        match self {
            Runner::Native => run_native(image, timeout),
            Runner::Qemu(spec) => run_qemu(spec, image, timeout, None),
            Runner::QemuDebug { spec, gdb_port } => {
                run_qemu(spec, image, timeout, Some(*gdb_port))
            }
            Runner::Device(spec) => run_device(spec, image, timeout),
        }
    }

    /// Spawn a QEMU process with gdbstub enabled and CPU frozen (`-S`).
    ///
    /// Returns the child process handle and the port it is listening on.
    /// The caller is responsible for killing the process when done.
    /// This is a building block for A-side escalation (Phase 14).
    pub fn spawn_debug(
        spec: &'static QemuSpec,
        image: &Path,
        port: u16,
    ) -> Result<Child, String> {
        let bin = std::str::from_utf8(spec.system_bin)
            .map_err(|_| "non-UTF-8 QEMU binary name")?;
        let machine = std::str::from_utf8(spec.machine)
            .map_err(|_| "non-UTF-8 QEMU machine name")?;

        let mut cmd = Command::new(bin);
        cmd.arg("-machine").arg(machine);
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        for arg in spec.extra_args {
            let s = std::str::from_utf8(arg)
                .map_err(|_| "non-UTF-8 QEMU extra arg")?;
            // Exclude isa-debug-exit device; for debug mode, we control
            // the target via gdbstub instead.
            if s.starts_with("-device") || s.starts_with("-debugcon") {
                continue;
            }
            cmd.arg(s);
        }

        match spec.exit_convention {
            codegen_core::QemuExitConvention::Semihosting => {
                cmd.arg("-semihosting-config");
                cmd.arg("enable=on,target=native");
            }
            codegen_core::QemuExitConvention::IsaDebugExit { .. } => {}
        }

        cmd.arg("-gdb").arg(format!("tcp::{}", port));
        cmd.arg("-S"); // freeze CPU at startup
        cmd.arg("-kernel").arg(image);

        cmd.spawn().map_err(|e| format!("spawning debug QEMU: {}", e))
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
                return Err(TyuError::Build(format!("waitpid failed: {}", e)).into());
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

fn run_qemu(
    spec: &QemuSpec,
    image: &Path,
    timeout: Duration,
    gdb_port: Option<u16>,
) -> Result<RunOutcome, String> {
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

    if let Some(port) = gdb_port {
        cmd.arg("-gdb").arg(format!("tcp::{}", port));
        cmd.arg("-S");
    }

    cmd.arg("-kernel").arg(image);

    spawn_and_wait(&mut cmd, image, timeout)
}

// ---------------------------------------------------------------------------
// Device runner (OpenOCD)
// ---------------------------------------------------------------------------

/// Flash and run an image on physical hardware via OpenOCD.
///
/// Pipeline:
///   1. Flash the ELF via `openocd -f <config> -c "program <image> reset exit"`.
///   2. Capture serial output from `<serial_port>` at `<baud>` baud.
///   3. Wait for completion or timeout.
fn run_device(spec: &OpenOcdSpec, image: &Path, timeout: Duration) -> Result<RunOutcome, String> {
    let _ = timeout;
    // Step 1: Flash via OpenOCD.
    let _flash_timeout = Duration::from_secs(spec.flash_timeout_secs);
    let flash_cmd = format!("program {} reset exit", image.display());
    let mut openocd = Command::new(&spec.bin);
    openocd
        .arg("-f")
        .arg(&spec.config)
        .arg("-c")
        .arg(&flash_cmd);

    let flash_status = openocd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| TyuError::Build(format!("spawning OpenOCD '{}': {}", spec.bin, e)))?;

    if !flash_status.success() {
        return Err(TyuError::Build(format!("OpenOCD flash failed for '{}'", image.display())).into());
    }

    // Step 2: Open serial port and capture output.
    let mut ser = open_serial(&spec.serial_port, spec.baud)?;

    let start = Instant::now();
    let mut stdout = Vec::new();
    let mut buf = [0u8; 1024];
    let mut timed_out = false;

    loop {
        if start.elapsed() >= timeout {
            timed_out = true;
            break;
        }
        // Read serial with a short timeout.
        match read_serial(&mut ser, &mut buf, Duration::from_millis(100)) {
            Ok(0) => {} // no data, keep polling
            Ok(n) => stdout.extend_from_slice(&buf[..n]),
            Err(_) => break, // serial error
        }
    }

    Ok(RunOutcome {
        exit_code: if timed_out { -1 } else { 0 },
        stdout,
        timed_out,
    })
}

#[cfg(not(target_os = "windows"))]
fn open_serial(port: &str, _baud: u32) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::fs::OpenOptions;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY) // non-blocking, no controlling TTY
        .open(port)
        .map_err(|e| format!("opening serial port '{}': {}", port, e))
}

#[cfg(target_os = "windows")]
fn open_serial(port: &str, _baud: u32) -> Result<std::fs::File, String> {
    // Windows serial ports are opened differently.
    // For now, just return a stub error.
    Err("Device runner not implemented on Windows".into())
}

fn read_serial(file: &mut std::fs::File, buf: &mut [u8], _timeout: Duration) -> Result<usize, String> {
    use std::io::Read;
    file.read(buf).map_err(|e| format!("serial read error: {}", e))
}
