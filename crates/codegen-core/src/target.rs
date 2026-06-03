/// LLVM-convention target triples supported by the compiler.
///
/// Each variant corresponds to an exact triple string that the user passes
/// via `--target`. The `parse` method is the sole place where a raw byte
/// string is converted to this type; all downstream code receives `Target`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    /// x86-64 Linux, System V ABI, glibc hosted.
    X86_64UnknownLinuxGnu,
    /// x86-64 bare-metal — no OS, QEMU system mode only.
    X86_64UnknownNone,
}

impl Target {
    /// Parse a target triple from a raw byte string.
    ///
    /// Returns `None` for any unrecognised triple so the caller can emit a
    /// clean diagnostic.
    pub fn parse(s: &[u8]) -> Option<Self> {
        match s {
            b"x86_64-unknown-linux-gnu" => Some(Self::X86_64UnknownLinuxGnu),
            b"x86_64-unknown-none" => Some(Self::X86_64UnknownNone),
            _ => None,
        }
    }

    /// The canonical LLVM triple string for this target.
    pub fn triple(self) -> &'static [u8] {
        match self {
            Self::X86_64UnknownLinuxGnu => b"x86_64-unknown-linux-gnu",
            Self::X86_64UnknownNone => b"x86_64-unknown-none",
        }
    }

    /// The `TargetSpec` for this target. This is the single source of truth
    /// for all target-dependent properties throughout the compiler.
    pub fn spec(self) -> &'static TargetSpec {
        match self {
            Self::X86_64UnknownLinuxGnu => &X86_64_UNKNOWN_LINUX_GNU,
            Self::X86_64UnknownNone => &X86_64_UNKNOWN_NONE,
        }
    }
}

// ---------------------------------------------------------------------------
// Supporting enumerations
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallingConv {
    /// System V AMD64 ABI — used by Linux x86-64.
    SysV64,
    /// ARM Procedure Call Standard (32-bit) — AAPCS.
    Aapcs32,
    /// Standard RISC-V calling convention (ILP32 / LP64).
    RiscV,
}

/// Which assembler is used to translate the text output to an object file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblerKind {
    /// Flat assembler (FASM). Used for x86-64 hosted targets.
    Fasm,
    /// GNU assembler targeting ARM bare-metal (`arm-none-eabi-as`).
    GasArm,
    /// GNU assembler targeting RISC-V bare-metal (`riscv32-unknown-elf-as`).
    GasRiscV,
}

/// The binary format produced by `--emit=obj`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    /// ELF64 relocatable object or executable.
    Elf64,
    /// ELF32 relocatable object or executable.
    Elf32,
    /// Raw flat binary image. No headers.
    FlatBin,
    /// Intel HEX record format. Used for flash programming.
    IntelHex,
}

/// Optional platform-level services a target's sysroot provides.
///
/// The harness queries this to skip fixtures that require absent capabilities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformCapability {
    /// Cooperative or preemptive task runtime present (`platform/linux`).
    TaskScheduler,
    /// Heap / region allocator present (`platform/mem` with live impl).
    DynamicAlloc,
    /// Channel IPC present (`platform/channel`).
    Channels,
}

/// How QEMU signals test pass/fail back to the host.
#[derive(Clone, Copy, Debug)]
pub enum QemuExitConvention {
    /// Guest writes N to `iobase`; QEMU exits with `(N << 1) | 1`.
    /// `host_pass_exit` is the QEMU process exit code meaning "all passed".
    IsaDebugExit { iobase: u16, host_pass_exit: i32 },
    /// ARM / RISC-V semihosting SYS_EXIT — value passed directly.
    Semihosting,
}

impl QemuExitConvention {
    /// The QEMU host process exit code that means "all tests passed".
    pub fn host_pass_exit(self) -> i32 {
        match self {
            Self::IsaDebugExit { host_pass_exit, .. } => host_pass_exit,
            Self::Semihosting => 0,
        }
    }
}

/// QEMU system-mode invocation parameters for a target.
pub struct QemuSpec {
    /// The QEMU system binary name, e.g. `b"qemu-system-x86_64"`.
    pub system_bin: &'static [u8],
    /// `-machine` argument value, e.g. `b"q35"`.
    pub machine: &'static [u8],
    /// Extra arguments inserted before `-kernel <image>`.
    pub extra_args: &'static [&'static [u8]],
    /// How guest communicates pass/fail to the host.
    pub exit_convention: QemuExitConvention,
}

// ---------------------------------------------------------------------------
// TargetSpec — the single source of truth for all target-dependent decisions
// ---------------------------------------------------------------------------

/// All properties that vary by target, in one place.
///
/// Obtain an instance via `Target::spec()`. Never scatter `if target == …`
/// checks through the codebase — query `TargetSpec` fields instead.
pub struct TargetSpec {
    /// Natural integer width in bits (32 or 64).
    /// Determines the type of builtins like `+`, `-`, `dup`.
    pub word_bits: u8,

    /// Pointer width in bits. May differ from `word_bits` on Harvard
    /// architectures. Determines the size of `ptr` and `ptr-mut`.
    pub pointer_bits: u8,

    pub endian: Endian,

    /// Hardware integer multiply is available.
    /// If false, the backend must emit a software multiply call.
    pub has_hardware_mul: bool,

    /// Hardware integer divide is available.
    /// If false, the backend must emit a software divide call.
    pub has_hardware_div: bool,

    /// Floating-point unit is present.
    pub has_fpu: bool,

    /// Minimum stack pointer alignment in bytes at a call boundary.
    pub stack_alignment_bytes: u8,

    /// Binary format produced by `--emit=obj`.
    pub output_format: OutputFormat,

    /// Assembler used to lower text assembly to an object file.
    pub assembler: AssemblerKind,

    pub calling_conv: CallingConv,

    /// The IR type name for the native integer word.
    /// `b"i64"` on 64-bit targets, `b"i32"` on 32-bit targets.
    /// Used to register arithmetic builtins with the correct type.
    pub native_int_ty: &'static [u8],

    /// Platform-level capabilities available via sysroot modules.
    /// Used by the test harness to filter fixtures.
    pub capabilities: &'static [PlatformCapability],

    /// QEMU system-mode parameters. `None` for host-native targets.
    pub qemu: Option<&'static QemuSpec>,
}

// ---------------------------------------------------------------------------
// Static target specifications
// ---------------------------------------------------------------------------

static X86_64_UNKNOWN_LINUX_GNU: TargetSpec = TargetSpec {
    word_bits: 64,
    pointer_bits: 64,
    endian: Endian::Little,
    has_hardware_mul: true,
    has_hardware_div: true,
    has_fpu: true,
    stack_alignment_bytes: 16,
    output_format: OutputFormat::Elf64,
    assembler: AssemblerKind::Fasm,
    calling_conv: CallingConv::SysV64,
    native_int_ty: b"i64",
    capabilities: &[
        PlatformCapability::TaskScheduler,
        PlatformCapability::DynamicAlloc,
        PlatformCapability::Channels,
    ],
    qemu: None,
};

static X86_64_NONE_QEMU_EXTRA_ARGS: [&[u8]; 8] = [
    b"-m",
    b"32M",
    b"-display",
    b"none",
    b"-device",
    b"isa-debug-exit,iobase=0x501,iosize=0x02",
    b"-debugcon",
    b"stdio",
];

static X86_64_UNKNOWN_NONE_QEMU: QemuSpec = QemuSpec {
    system_bin: b"qemu-system-x86_64",
    machine: b"q35",
    extra_args: &X86_64_NONE_QEMU_EXTRA_ARGS,
    exit_convention: QemuExitConvention::IsaDebugExit {
        iobase: 0x501,
        // guest writes 0 → QEMU exits with (0<<1)|1 = 1
        host_pass_exit: 1,
    },
};

static X86_64_UNKNOWN_NONE: TargetSpec = TargetSpec {
    word_bits: 64,
    pointer_bits: 64,
    endian: Endian::Little,
    has_hardware_mul: true,
    has_hardware_div: true,
    has_fpu: true,
    stack_alignment_bytes: 16,
    output_format: OutputFormat::Elf64,
    assembler: AssemblerKind::Fasm,
    calling_conv: CallingConv::SysV64,
    native_int_ty: b"i64",
    capabilities: &[],
    qemu: Some(&X86_64_UNKNOWN_NONE_QEMU),
};
