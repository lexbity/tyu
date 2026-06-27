//! Pluggable backend runner abstraction.
//!
//! Executes a built image: natively (hosted target), under QEMU (bare-metal
//! target), or on a physical device via OpenOCD/probe-rs.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::TyuError;
use codegen_core::target::QemuSpec;
use codegen_core::Target;

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

/// How QEMU debug launches should start the guest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QemuDebugStart {
    FrozenAtReset,
    RunImmediately,
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
    pub fn run(&self, image: &Path, timeout: Duration) -> Result<RunOutcome, TyuError> {
        match self {
            Runner::Native => run_native(image, timeout),
            Runner::Qemu(spec) => run_qemu(spec, image, timeout, None),
            Runner::QemuDebug { spec, gdb_port } => run_qemu(spec, image, timeout, Some(*gdb_port)),
            Runner::Device(spec) => run_device(spec, image, timeout),
        }
    }

    /// Run a legacy static artifact. Static bare-metal builds may keep the
    /// shipped artifact as `.lmod` while QEMU executes the companion ELF it was
    /// packed from. Dynamic firmware paths must call `run` with the firmware ELF.
    pub fn run_static_artifact(
        &self,
        image: &Path,
        timeout: Duration,
    ) -> Result<RunOutcome, TyuError> {
        let exec_image = resolve_static_image(image)?;
        self.run(&exec_image, timeout)
    }

    /// Spawn a QEMU process with gdbstub enabled.
    ///
    /// Returns the child process handle and the port it is listening on.
    /// The caller is responsible for killing the process when done.
    /// This is a building block for A-side escalation (Phase 14).
    pub fn spawn_debug(
        spec: &'static QemuSpec,
        image: &Path,
        port: u16,
        mode: QemuDebugStart,
    ) -> Result<Child, TyuError> {
        let mut cmd = build_qemu_command(spec, image, Some(port), Some(mode))?;
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        cmd.spawn()
            .map_err(|e| TyuError::Runner(format!("spawning debug QEMU: {}", e)))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_static_image(image: &Path) -> Result<PathBuf, TyuError> {
    if image.extension().and_then(|s| s.to_str()) != Some("lmod") {
        return Ok(image.to_path_buf());
    }
    let mut candidates = vec![image.with_extension("elf")];
    if let Some(dir) = image.parent() {
        candidates.push(dir.join("image.elf"));
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("elf") {
                    candidates.push(path);
                }
            }
        }
    }
    candidates.into_iter().find(|p| p.exists()).ok_or_else(|| {
        TyuError::Runner(format!(
            "execution image '{}' has no ELF sibling or image.elf companion",
            image.display()
        ))
    })
}

/// Spawn a process and wait with timeout, capturing stdout.
fn spawn_and_wait(
    cmd: &mut Command,
    image: &Path,
    timeout: Duration,
) -> Result<RunOutcome, TyuError> {
    // Capture both streams: bare-metal targets emit their framed diagnostic
    // output over semihosting, which QEMU writes to *stderr*, while the hosted /
    // debugcon path writes to stdout.  Merging both keeps the runner
    // target-agnostic; the frame parser resyncs on record markers and ignores
    // any interleaved QEMU diagnostics.
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| TyuError::Runner(format!("spawning '{}': {}", image.display(), e)))?;

    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| TyuError::Runner("failed to capture stdout".into()))?;
    let stdout_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout_pipe, &mut buf);
        buf
    });

    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| TyuError::Runner("failed to capture stderr".into()))?;
    let stderr_handle = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr_pipe, &mut buf);
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
                    let mut out = stdout_handle.join().unwrap_or_default();
                    out.extend(stderr_handle.join().unwrap_or_default());
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
                let _stderr = stderr_handle.join().unwrap_or_default();
                return Err(TyuError::Build(format!("waitpid failed: {}", e)).into());
            }
        }
    };

    let mut stdout = stdout_handle.join().unwrap_or_default();
    stdout.extend(stderr_handle.join().unwrap_or_default());
    Ok(RunOutcome {
        exit_code,
        stdout,
        timed_out: false,
    })
}

// ---------------------------------------------------------------------------
// Native runner
// ---------------------------------------------------------------------------

fn run_native(image: &Path, timeout: Duration) -> Result<RunOutcome, TyuError> {
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
) -> Result<RunOutcome, TyuError> {
    let mut cmd = build_qemu_command(
        spec,
        image,
        gdb_port,
        gdb_port.map(|_| QemuDebugStart::FrozenAtReset),
    )?;
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
fn run_device(spec: &OpenOcdSpec, image: &Path, timeout: Duration) -> Result<RunOutcome, TyuError> {
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
        return Err(
            TyuError::Build(format!("OpenOCD flash failed for '{}'", image.display())).into(),
        );
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
fn open_serial(port: &str, _baud: u32) -> Result<std::fs::File, TyuError> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY) // non-blocking, no controlling TTY
        .open(port)
        .map_err(|e| TyuError::Runner(format!("opening serial port '{}': {}", port, e)))
}

#[cfg(target_os = "windows")]
fn open_serial(port: &str, _baud: u32) -> Result<std::fs::File, TyuError> {
    // Windows serial ports are opened differently.
    // For now, just return a stub error.
    Err("Device runner not implemented on Windows".into())
}

fn read_serial(
    file: &mut std::fs::File,
    buf: &mut [u8],
    _timeout: Duration,
) -> Result<usize, TyuError> {
    use std::io::Read;
    file.read(buf)
        .map_err(|e| TyuError::Runner(format!("serial read error: {}", e)))
}

fn build_qemu_command(
    spec: &QemuSpec,
    image: &Path,
    gdb_port: Option<u16>,
    debug_start: Option<QemuDebugStart>,
) -> Result<Command, TyuError> {
    if debug_start.is_some() && gdb_port.is_none() {
        return Err(TyuError::Runner(
            "debug QEMU launch requires a gdb port".into(),
        ));
    }

    let bin = std::str::from_utf8(spec.system_bin)
        .map_err(|_| TyuError::Runner("non-UTF-8 QEMU binary name".into()))?;
    let machine = std::str::from_utf8(spec.machine)
        .map_err(|_| TyuError::Runner("non-UTF-8 QEMU machine name".into()))?;
    let mut cmd = Command::new(bin);
    cmd.arg("-machine").arg(machine);

    let debug_mode = gdb_port.is_some() || debug_start.is_some();
    let extra_args = if debug_mode {
        filtered_debug_extra_args(spec.extra_args)?
    } else {
        collect_qemu_extra_args(spec.extra_args)?
    };
    for arg in extra_args {
        cmd.arg(arg);
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
        if debug_start != Some(QemuDebugStart::RunImmediately) {
            cmd.arg("-S");
        }
    }

    cmd.arg("-kernel").arg(image);
    Ok(cmd)
}

fn collect_qemu_extra_args(extra_args: &[&[u8]]) -> Result<Vec<String>, TyuError> {
    extra_args
        .iter()
        .map(|arg| {
            std::str::from_utf8(arg)
                .map(|s| s.to_owned())
                .map_err(|_| TyuError::Runner("non-UTF-8 QEMU extra arg".into()))
        })
        .collect()
}

fn filtered_debug_extra_args(extra_args: &[&[u8]]) -> Result<Vec<String>, TyuError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < extra_args.len() {
        let arg = std::str::from_utf8(extra_args[i])
            .map_err(|_| TyuError::Runner("non-UTF-8 QEMU extra arg".into()))?;
        match arg {
            "-device" => {
                let value = std::str::from_utf8(
                    extra_args
                        .get(i + 1)
                        .ok_or_else(|| TyuError::Runner("missing -device value".into()))?,
                )
                .map_err(|_| TyuError::Runner("non-UTF-8 QEMU extra arg".into()))?;
                if value.starts_with("isa-debug-exit") {
                    i += 2;
                    continue;
                }
                out.push(arg.to_owned());
                out.push(value.to_owned());
                i += 2;
            }
            "-debugcon" => {
                let _value = std::str::from_utf8(
                    extra_args
                        .get(i + 1)
                        .ok_or_else(|| TyuError::Runner("missing -debugcon value".into()))?,
                )
                .map_err(|_| TyuError::Runner("non-UTF-8 QEMU extra arg".into()))?;
                i += 2;
            }
            _ => {
                out.push(arg.to_owned());
                i += 1;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegen_core::Target;
    use std::path::Path;

    fn qemu_args(
        target: Target,
        mode: Option<QemuDebugStart>,
        gdb_port: Option<u16>,
    ) -> Vec<String> {
        let spec = target.spec().qemu.expect("target should support qemu");
        build_qemu_command(spec, Path::new("/tmp/image.elf"), gdb_port, mode)
            .expect("build qemu command")
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn debug_args_frozen_has_dash_s() {
        let args = qemu_args(
            Target::X86_64UnknownNone,
            Some(QemuDebugStart::FrozenAtReset),
            Some(1234),
        );
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-gdb" && w[1] == "tcp::1234"));
        assert!(args.iter().any(|arg| arg == "-S"));
    }

    #[test]
    fn debug_args_running_omits_dash_s() {
        let args = qemu_args(
            Target::X86_64UnknownNone,
            Some(QemuDebugStart::RunImmediately),
            Some(1234),
        );
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-gdb" && w[1] == "tcp::1234"));
        assert!(!args.iter().any(|arg| arg == "-S"));
    }

    #[test]
    fn debug_args_x86_strip_debug_exit_pair() {
        let args = qemu_args(
            Target::X86_64UnknownNone,
            Some(QemuDebugStart::FrozenAtReset),
            Some(1234),
        );
        assert!(!args.iter().any(|arg| arg == "-device"));
        assert!(!args
            .iter()
            .any(|arg| arg == "isa-debug-exit,iobase=0x501,iosize=0x02"));
    }

    #[test]
    fn debug_args_x86_strip_debugcon_pair() {
        let args = qemu_args(
            Target::X86_64UnknownNone,
            Some(QemuDebugStart::FrozenAtReset),
            Some(1234),
        );
        assert!(!args.iter().any(|arg| arg == "-debugcon"));
        assert!(!args.iter().any(|arg| arg == "stdio"));
    }

    #[test]
    fn debug_args_arm_preserve_nographic() {
        let args = qemu_args(
            Target::ArmV7MUnknownNone,
            Some(QemuDebugStart::FrozenAtReset),
            Some(1234),
        );
        assert!(args.iter().any(|arg| arg == "-nographic"));
    }

    #[test]
    fn debug_args_riscv_preserve_bios_none() {
        let args = qemu_args(
            Target::RiscV32UnknownNone,
            Some(QemuDebugStart::FrozenAtReset),
            Some(1234),
        );
        assert!(args.windows(2).any(|w| w[0] == "-bios" && w[1] == "none"));
        assert!(args.iter().any(|arg| arg == "-nographic"));
    }
}
