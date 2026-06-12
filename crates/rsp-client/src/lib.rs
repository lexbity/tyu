//! Minimal GDB Remote Serial Protocol (RSP) client.
//!
//! Speaks the RSP subset required by the A-side escalation debugger
//! (Phase 14+): `m`, `g`, `p`, `Z0`, `z0`, `c`, `s`.
//!
//! The client connects to QEMU's `-gdb tcp::<PORT>` stub and exchanges
//! packets over the RSP framing layer (`$<data>#<csum>`).

pub mod regs;

use std::io::{self, Read, Write};
use std::net::{TcpStream, TcpListener};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default connection timeout.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Response timeout for individual RSP commands.
pub const RSP_TIMEOUT: Duration = Duration::from_secs(10);

/// Retry interval for connect_retry.
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// Maximum retry attempts for connect_retry.
const CONNECT_RETRY_MAX: u32 = 100;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Bind to `:0` to get an ephemeral port, drop the listener, and return
/// the port number.  The port is free to be reused by a subsequent bind.
///
/// There is a TOCTOU window between dropping the listener and the caller
/// using the port — connect_retry handles this by retrying on failure.
pub fn ephemeral_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .expect("ephemeral_port: bind to :0 failed");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// Connect to `host:port` with retry up to `CONNECT_RETRY_MAX` times.
/// Returns `Ok(stream)` on success or `Err` after all retries are exhausted.
pub fn connect_retry(host: &str, port: u16) -> io::Result<TcpStream> {
    let addr = format!("{host}:{port}");
    let deadline = Instant::now() + CONNECT_TIMEOUT * 2;
    for attempt in 0..CONNECT_RETRY_MAX {
        match TcpStream::connect_timeout(
            &addr.as_str().parse().map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?,
            CONNECT_RETRY_INTERVAL,
        ) {
            Ok(stream) => {
                stream.set_read_timeout(Some(RSP_TIMEOUT))?;
                stream.set_write_timeout(Some(RSP_TIMEOUT))?;
                return Ok(stream);
            }
            Err(_) if Instant::now() >= deadline => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("connect_retry: timed out after {attempt} attempts"),
                ));
            }
            Err(_) => {
                std::thread::sleep(CONNECT_RETRY_INTERVAL);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("connect_retry: exhausted {CONNECT_RETRY_MAX} attempts"),
    ))
}

// ---------------------------------------------------------------------------
// RSP packet helpers
// ---------------------------------------------------------------------------

/// Compute the RSP checksum (modulo-256 sum of all bytes).
pub fn rsp_checksum(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |sum, &b| sum.wrapping_add(b))
}

/// Encode data into an RSP packet: `$<data>#<csum_hex>`.
pub fn encode_packet(data: &[u8]) -> Vec<u8> {
    let csum = rsp_checksum(data);
    let mut pkt = Vec::with_capacity(data.len() + 4);
    pkt.push(b'$');
    pkt.extend_from_slice(data);
    pkt.extend_from_slice(b"#");
    pkt.push(hex_nibble(csum >> 4));
    pkt.push(hex_nibble(csum & 0xf));
    pkt
}

fn hex_nibble(v: u8) -> u8 {
    if v < 10 { b'0' + v } else { b'a' + v - 10 }
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decode a hex string into bytes.
pub fn hex_decode(hex: &[u8]) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for chunk in hex.chunks(2) {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

/// Encode bytes as a lowercase hex string.
pub fn hex_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2);
    for &b in data {
        out.push(hex_nibble(b >> 4));
        out.push(hex_nibble(b & 0xf));
    }
    out
}

/// Parse an RSP packet from a raw byte buffer.
///
/// Returns `Some((payload, consumed_bytes))` on success, or `None` if
/// the buffer does not contain a complete valid packet.
pub fn parse_packet(buf: &[u8]) -> Option<(&[u8], usize)> {
    let start = buf.iter().position(|&b| b == b'$')?;
    let end = buf[start..].iter().position(|&b| b == b'#')?;
    let csum_start = start + end + 1;
    if csum_start + 2 > buf.len() {
        return None;
    }
    let data = &buf[start + 1..start + end];
    let csum_hi = hex_val(buf[csum_start])?;
    let csum_lo = hex_val(buf[csum_start + 1])?;
    let expected = (csum_hi << 4) | csum_lo;
    let actual = rsp_checksum(data);
    if expected != actual {
        return None;
    }
    let consumed = csum_start + 2 - start;
    Some((data, consumed))
}

// ---------------------------------------------------------------------------
// RSP Client
// ---------------------------------------------------------------------------

/// A TCP connection to a QEMU gdbstub speaking the Remote Serial Protocol.
pub struct RspClient {
    stream: TcpStream,
    recv_buf: Vec<u8>,
}

impl RspClient {
    /// Connect to a QEMU gdbstub at `host:port`.
    ///
    /// The stub must already be listening (QEMU started with `-gdb tcp::PORT`)
    /// and the CPU should be stopped (`-S` flag).
    pub fn connect(host: &str, port: u16) -> io::Result<Self> {
        let addr = format!("{}:{}", host, port);
        let stream = TcpStream::connect_timeout(
            &addr.parse().map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?,
            CONNECT_TIMEOUT,
        )?;
        stream.set_read_timeout(Some(RSP_TIMEOUT))?;
        stream.set_write_timeout(Some(RSP_TIMEOUT))?;
        Ok(Self {
            stream,
            recv_buf: Vec::with_capacity(4096),
        })
    }

    /// Send a raw RSP packet and await the `+` acknowledgment.
    fn send_raw(&mut self, payload: &[u8]) -> io::Result<()> {
        let pkt = encode_packet(payload);
        self.stream.write_all(&pkt)?;
        self.wait_ack()
    }

    /// Send a packet and read the response (skipping the leading `+` ack
    /// if QEMU sends one before the response).
    fn send_and_recv(&mut self, payload: &[u8]) -> io::Result<Vec<u8>> {
        let pkt = encode_packet(payload);
        self.stream.write_all(&pkt)?;

        // Read until we have a complete response packet.
        loop {
            // Discard any leading '+' or '-' bytes.
            while let Some(&b'+') = self.recv_buf.first() {
                self.recv_buf.remove(0);
            }
            if self.recv_buf.first() == Some(&b'-') {
                self.recv_buf.clear();
                self.stream.write_all(&pkt)?;
                continue;
            }

            // Try to find a complete packet in the buffer.
            let (resp, consumed) = {
                let buf = &self.recv_buf;
                match parse_packet(buf) {
                    Some((r, c)) => (r.to_vec(), c),
                    None => {
                        // Read more data.
                        let mut tmp = [0u8; 1024];
                        let n = self.stream.read(&mut tmp)?;
                        if n == 0 {
                            return Err(io::Error::new(
                                io::ErrorKind::ConnectionReset,
                                "connection closed",
                            ));
                        }
                        self.recv_buf.extend_from_slice(&tmp[..n]);
                        continue;
                    }
                }
            };
            self.recv_buf.drain(..consumed);
            self.stream.write_all(b"+")?;
            return Ok(resp);
        }
    }

    /// Wait for a single `+` acknowledgment byte.
    fn wait_ack(&mut self) -> io::Result<()> {
        loop {
            if let Some(pos) = self.recv_buf.iter().position(|&b| b == b'+') {
                self.recv_buf.drain(..=pos);
                return Ok(());
            }
            if self.recv_buf.iter().any(|&b| b == b'-') {
                // NAK — caller should handle via resend logic.
                self.recv_buf.clear();
                return Err(io::Error::new(io::ErrorKind::Other, "NAK"));
            }
            let mut tmp = [0u8; 1];
            if self.stream.read(&mut tmp)? == 0 {
                return Err(io::Error::new(io::ErrorKind::ConnectionReset, "connection closed"));
            }
            self.recv_buf.push(tmp[0]);
        }
    }

    // -------------------------------------------------------------------
    // High-level RSP commands
    // -------------------------------------------------------------------

    /// Read all registers.  Returns raw register data in GDB's target
    /// register order (architecture-dependent byte layout).
    pub fn read_registers(&mut self) -> io::Result<Vec<u8>> {
        let resp = self.send_and_recv(b"g")?;
        if resp == b"E01" || resp == b"E" {
            return Err(io::Error::new(io::ErrorKind::Other, "g (read registers) failed"));
        }
        hex_decode(&resp).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "g response is not valid hex")
        })
    }

    /// Read a single register by GDB register number.
    pub fn read_register(&mut self, reg: u8) -> io::Result<Vec<u8>> {
        let cmd = format!("p{:02x}", reg);
        let resp = self.send_and_recv(cmd.as_bytes())?;
        if resp == b"E01" || resp == b"E" {
            return Err(io::Error::new(io::ErrorKind::Other, format!("p{:02x} failed", reg)));
        }
        hex_decode(&resp).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "p response is not valid hex")
        })
    }

    /// Read memory at `addr` for `len` bytes.
    pub fn read_memory(&mut self, addr: u64, len: usize) -> io::Result<Vec<u8>> {
        let cmd = format!("m{:x},{:x}", addr, len);
        let resp = self.send_and_recv(cmd.as_bytes())?;
        if resp == b"E01" || resp == b"E" {
            return Err(io::Error::new(io::ErrorKind::Other, format!("m failed at {:#x}", addr)));
        }
        hex_decode(&resp).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "m response is not valid hex")
        })
    }

    /// Write memory at `addr` from `data` bytes.
    #[allow(dead_code)]
    pub fn write_memory(&mut self, addr: u64, data: &[u8]) -> io::Result<()> {
        let hex = hex_encode(data);
        let hex_str = core::str::from_utf8(&hex).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "write data not valid utf-8")
        })?;
        let cmd = format!("M{:x},{:x}:{}", addr, data.len(), hex_str);
        let resp = self.send_and_recv(cmd.as_bytes())?;
        if resp == b"OK" {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::Other, format!("M failed at {:#x}", addr)))
        }
    }

    /// Insert a software breakpoint at `addr`.
    pub fn set_breakpoint(&mut self, addr: u64) -> io::Result<()> {
        let cmd = format!("Z0,{:x},1", addr);
        let resp = self.send_and_recv(cmd.as_bytes())?;
        if resp == b"OK" {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::Other, format!("Z0 failed at {:#x}", addr)))
        }
    }

    /// Remove a software breakpoint at `addr`.
    #[allow(dead_code)]
    pub fn remove_breakpoint(&mut self, addr: u64) -> io::Result<()> {
        let cmd = format!("z0,{:x},1", addr);
        let resp = self.send_and_recv(cmd.as_bytes())?;
        if resp == b"OK" {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::Other, format!("z0 failed at {:#x}", addr)))
        }
    }

    /// Continue execution.  Returns when the target stops (breakpoint hit,
    /// trap, or single-step completion).
    pub fn continue_exec(&mut self) -> io::Result<()> {
        let _resp = self.send_and_recv(b"c")?;
        Ok(())
    }

    /// Single-step one instruction.  Returns when the target stops.
    #[allow(dead_code)]
    pub fn single_step(&mut self) -> io::Result<()> {
        let _resp = self.send_and_recv(b"s")?;
        Ok(())
    }

    /// Send a "query" packet (`q<name>:...`).
    #[allow(dead_code)]
    pub fn query(&mut self, name: &[u8]) -> io::Result<Vec<u8>> {
        let mut cmd = Vec::with_capacity(name.len() + 1);
        cmd.push(b'q');
        cmd.extend_from_slice(name);
        self.send_and_recv(&cmd)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------
    // Packet encoding / decoding
    // -------------------------------------------------------------------

    #[test]
    fn checksum_empty() {
        assert_eq!(rsp_checksum(b""), 0);
    }

    #[test]
    fn checksum_known() {
        // "c" -> 0x63
        assert_eq!(rsp_checksum(b"c"), 0x63);
    }

    #[test]
    fn checksum_multibyte() {
        let data = b"g";
        assert_eq!(rsp_checksum(data), b'g');
    }

    #[test]
    fn encode_packet_format() {
        let pkt = encode_packet(b"c");
        // $c#<csum>
        assert_eq!(pkt[0], b'$');
        assert_eq!(pkt[1], b'c');
        assert_eq!(pkt[2], b'#');
        // checksum of "c" = 0x63
        assert_eq!(pkt[3], b'6');
        assert_eq!(pkt[4], b'3');
    }

    #[test]
    fn encode_packet_with_data() {
        let data = b"g";
        let pkt = encode_packet(data);
        let csum = rsp_checksum(b"g");
        assert_eq!(pkt, [b'$', b'g', b'#', hex_nibble(csum >> 4), hex_nibble(csum & 0xf)]);
    }

    #[test]
    fn parse_packet_valid() {
        let pkt = encode_packet(b"OK");
        let (payload, consumed) = parse_packet(&pkt).unwrap();
        assert_eq!(payload, b"OK");
        assert_eq!(consumed, pkt.len());
    }

    #[test]
    fn parse_packet_with_garbage_before() {
        let mut buf = b"hello ".to_vec();
        buf.extend_from_slice(&encode_packet(b"OK"));
        let (payload, consumed) = parse_packet(&buf).unwrap();
        assert_eq!(payload, b"OK");
        // consumed is relative to the start of the '$'
        assert_eq!(consumed, encode_packet(b"OK").len());
    }

    #[test]
    fn parse_packet_bad_checksum() {
        let mut pkt = encode_packet(b"OK");
        let last = pkt.len() - 1;
        pkt[last] ^= 1;
        assert!(parse_packet(&pkt).is_none());
    }

    #[test]
    fn parse_packet_truncated() {
        assert!(parse_packet(b"$OK").is_none());
        assert!(parse_packet(b"").is_none());
    }

    #[test]
    fn parse_packet_no_dollar() {
        assert!(parse_packet(b"OK#00").is_none());
    }

    // -------------------------------------------------------------------
    // hex encoding / decoding
    // -------------------------------------------------------------------

    #[test]
    fn hex_encode_roundtrip() {
        let data = b"Hello\x00\xff";
        let enc = hex_encode(data);
        let dec = hex_decode(&enc).unwrap();
        assert_eq!(dec, data);
    }

    #[test]
    fn hex_decode_rejects_odd_length() {
        assert!(hex_decode(b"a").is_none());
    }

    #[test]
    fn hex_decode_rejects_invalid_chars() {
        assert!(hex_decode(b"xx").is_none());
    }

    #[test]
    fn hex_encode_zero() {
        assert_eq!(hex_encode(b"\x00"), b"00");
    }

    // -------------------------------------------------------------------
    // Integration tests — x86_64, ARM, RISC-V (gated on QEMU availability)
    // -------------------------------------------------------------------

    /// Helper: launch QEMU, connect RSP client via ephemeral port with
    /// connect_retry, run the roundtrip, kill QEMU, return result.
    fn qemu_rsp_roundtrip(
        qemu_bin: &str,
        qemu_args: &[&str],
        kernel_path: &std::path::Path,
        pc_reg: u8,
    ) -> Result<(), io::Error> {
        let port = ephemeral_port();

        let mut qemu = std::process::Command::new(qemu_bin);
        qemu.args(qemu_args);
        qemu.arg("-gdb").arg(format!("tcp::{port}"));
        qemu.arg("-S");
        qemu.arg("-kernel").arg(kernel_path);
        qemu.stdout(std::process::Stdio::null());
        qemu.stderr(std::process::Stdio::null());

        let mut child = qemu.spawn().map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("qemu spawn failed: {e}"))
        })?;

        let stream = connect_retry("127.0.0.1", port).map_err(|e| {
            let _ = child.kill(); let _ = child.wait(); e
        })?;
        let mut client = RspClient { stream, recv_buf: Vec::with_capacity(4096) };

        let result = (|| -> io::Result<()> {
            let pc_raw = client.read_register(pc_reg)?;
            assert!(!pc_raw.is_empty(), "PC must be readable");
            let pc_val = match pc_raw.len() {
                4 => u64::from_le_bytes([pc_raw[0], pc_raw[1], pc_raw[2], pc_raw[3], 0, 0, 0, 0]),
                8 => u64::from_le_bytes(pc_raw[..8].try_into().unwrap()),
                _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected PC width")),
            };
            let mem = client.read_memory(pc_val, 2)?;
            assert!(!mem.is_empty(), "memory at PC must be readable");
            Ok(())
        })();

        let _ = child.kill();
        let _ = child.wait();
        result
    }

    /// Build a minimal PVH ELF that loops forever, suitable for
    /// `qemu-system-x86_64 -kernel`.
    fn try_build_infinite_loop_elf(out_dir: &std::path::Path) -> Option<std::path::PathBuf> {
        let asm = out_dir.join("loop.asm");
        let elf = out_dir.join("loop.elf");
        // Minimal 64-bit PVH ELF: the Xen note tells QEMU the 32-bit entry,
        // which sets up long mode and jumps to the 64-bit _start.
        // Pattern derived from the working runtime.x86_64.asm.
        std::fs::write(
            &asm,
            b"format ELF64 executable\n\
              entry _start32\n\
              section '.note.Xen' align 4\n\
              dd 4\n  dd 4\n  dd 18\n  db 'Xen',0\n  dd _start32\n\
              section '.text' executable\n\
              use32\n_start32:\n  jmp dword 0x08:_start64\n\
              use64\n_start64:\n_start:\n  jmp _start\n",
        )
        .ok()?;
        let status = std::process::Command::new("fasm")
            .args([asm.to_str().unwrap(), elf.to_str().unwrap()])
            .status()
            .ok()?;
        if !status.success() {
            eprintln!("note: fasm failed to build infinite-loop ELF for RSP integration test");
            return None;
        }
        Some(elf)
    }

    #[test]
    fn qemu_read_register_and_memory() {
        let tools = ["qemu-system-x86_64", "fasm"];
        let missing: Vec<&str> = tools.iter().filter(|t| !tool_available(t)).copied().collect();
        if !missing.is_empty() {
            if std::env::var("CI").is_ok() {
                panic!("RSP integration test requires: {}", missing.join(", "));
            }
            eprintln!("SKIP: qemu RSP integration test (missing: {})", missing.join(", "));
            return;
        }

        let dir = temp_dir("rsp_qemu_test");
        let elf = match try_build_infinite_loop_elf(&dir) {
            Some(e) => e,
            None => {
                eprintln!("SKIP: could not build test ELF (fasm issue)");
                return;
            }
        };

        let qemu_args = [
            "-machine", "q35",
            "-m", "32M",
            "-display", "none",
            "-device", "isa-debug-exit,iobase=0x501,iosize=0x02",
        ];
        let result = qemu_rsp_roundtrip("qemu-system-x86_64", &qemu_args, &elf, regs::x86_64::RIP);
        if let Err(e) = result {
            panic!("RSP integration test failed: {}", e);
        }
    }

    /// Build a minimal ARM Thumb infinite-loop ELF (Cortex-M3, lm3s6965evb).
    fn try_build_arm_loop_elf(out_dir: &std::path::Path) -> Option<std::path::PathBuf> {
        let asm = out_dir.join("loop.s");
        let obj = out_dir.join("loop.o");
        let elf = out_dir.join("loop.elf");
        // Minimal vector table + infinite loop for lm3s6965evb.
        std::fs::write(
            &asm,
            b".syntax unified\n.thumb\n\n.section .vectors,\"ax\"\n\
              .word _estack\n.word _start + 1\n.space 0x100 - 8, 0\n\n\
              .section .text,\"ax\"\n.globl _start\n.type _start,%function\n_start:\n  b _start\n\n\
              .section .bss\n.space 0x1000\n_estack:\n",
        )
        .ok()?;
        let status = std::process::Command::new("arm-none-eabi-as")
            .args(["-mcpu=cortex-m3", "-mthumb", asm.to_str().unwrap(), "-o", obj.to_str().unwrap()])
            .status()
            .ok()?;
        if !status.success() {
            eprintln!("note: arm-none-eabi-as failed to build infinite-loop ELF");
            return None;
        }
        let status = std::process::Command::new("arm-none-eabi-ld")
            .args(["-Ttext=0x0", obj.to_str().unwrap(), "-o", elf.to_str().unwrap()])
            .status()
            .ok()?;
        if !status.success() {
            eprintln!("note: arm-none-eabi-ld failed");
            return None;
        }
        Some(elf)
    }

    /// Build a minimal RISC-V infinite-loop ELF (qemu-system-riscv32 virt machine).
    fn try_build_riscv_loop_elf(out_dir: &std::path::Path) -> Option<std::path::PathBuf> {
        let asm = out_dir.join("loop.s");
        let obj = out_dir.join("loop.o");
        let elf = out_dir.join("loop.elf");
        std::fs::write(&asm, b".globl _start\n_start:\n  j _start\n").ok()?;
        let status = std::process::Command::new("riscv64-unknown-elf-as")
            .args(["-march=rv32i", "-mabi=ilp32", asm.to_str().unwrap(), "-o", obj.to_str().unwrap()])
            .status()
            .ok()?;
        if !status.success() {
            eprintln!("note: riscv64-unknown-elf-as failed to build infinite-loop ELF");
            return None;
        }
        let status = std::process::Command::new("riscv64-unknown-elf-ld")
            .args(["-Ttext=0x80000000", obj.to_str().unwrap(), "-o", elf.to_str().unwrap()])
            .status()
            .ok()?;
        if !status.success() {
            eprintln!("note: riscv64-unknown-elf-ld failed");
            return None;
        }
        Some(elf)
    }

    #[test]
    fn arm_qemu_read_register_and_memory() {
        let tools = ["qemu-system-arm", "arm-none-eabi-as", "arm-none-eabi-ld"];
        let missing: Vec<&str> = tools.iter().filter(|t| !tool_available(t)).copied().collect();
        if !missing.is_empty() {
            if std::env::var("CI").is_ok() {
                panic!("ARM RSP integration test requires: {}", missing.join(", "));
            }
            eprintln!("SKIP: ARM RSP integration test (missing: {})", missing.join(", "));
            return;
        }

        let dir = temp_dir("rsp_arm_test");
        let elf = match try_build_arm_loop_elf(&dir) {
            Some(e) => e,
            None => { eprintln!("SKIP: could not build ARM test ELF"); return; }
        };

        let qemu_args = [
            "-machine", "lm3s6965evb",
            "-semihosting-config", "enable=on,target=native",
            "-nographic",
        ];
        let result = qemu_rsp_roundtrip("qemu-system-arm", &qemu_args, &elf, regs::arm::PC);
        if let Err(e) = result {
            panic!("ARM RSP integration test failed: {}", e);
        }
    }

    #[test]
    fn riscv_qemu_read_register_and_memory() {
        let tools = ["qemu-system-riscv32", "riscv64-unknown-elf-as", "riscv64-unknown-elf-ld"];
        let missing: Vec<&str> = tools.iter().filter(|t| !tool_available(t)).copied().collect();
        if !missing.is_empty() {
            if std::env::var("CI").is_ok() {
                panic!("RISC-V RSP integration test requires: {}", missing.join(", "));
            }
            eprintln!("SKIP: RISC-V RSP integration test (missing: {})", missing.join(", "));
            return;
        }

        let dir = temp_dir("rsp_riscv_test");
        let elf = match try_build_riscv_loop_elf(&dir) {
            Some(e) => e,
            None => { eprintln!("SKIP: could not build RISC-V test ELF"); return; }
        };

        let qemu_args = [
            "-machine", "virt",
            "-semihosting-config", "enable=on,target=native",
            "-nographic",
        ];
        let result = qemu_rsp_roundtrip("qemu-system-riscv32", &qemu_args, &elf, regs::riscv::PC);
        if let Err(e) = result {
            panic!("RISC-V RSP integration test failed: {}", e);
        }
    }

    // -------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------

    fn tool_available(name: &str) -> bool {
        std::process::Command::new("which")
            .arg(name)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join("rsp_client_tests")
            .join(format!("{}_{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
