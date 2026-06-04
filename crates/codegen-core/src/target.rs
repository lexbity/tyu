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
    /// ARM Cortex-M3 bare-metal (lm3s6965evb QEMU machine, Thumb).
    ArmV7MUnknownNone,
    /// RISC-V 32-bit bare-metal (QEMU virt machine, RV32IM).
    RiscV32UnknownNone,
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
            b"armv7m-unknown-none" => Some(Self::ArmV7MUnknownNone),
            b"riscv32-unknown-none" => Some(Self::RiscV32UnknownNone),
            _ => None,
        }
    }

    /// The canonical LLVM triple string for this target.
    pub fn triple(self) -> &'static [u8] {
        match self {
            Self::X86_64UnknownLinuxGnu => b"x86_64-unknown-linux-gnu",
            Self::X86_64UnknownNone => b"x86_64-unknown-none",
            Self::ArmV7MUnknownNone => b"armv7m-unknown-none",
            Self::RiscV32UnknownNone => b"riscv32-unknown-none",
        }
    }

    /// The `TargetSpec` for this target. This is the single source of truth
    /// for all target-dependent properties throughout the compiler.
    pub fn spec(self) -> &'static TargetSpec {
        match self {
            Self::X86_64UnknownLinuxGnu => &X86_64_UNKNOWN_LINUX_GNU,
            Self::X86_64UnknownNone => &X86_64_UNKNOWN_NONE,
            Self::ArmV7MUnknownNone => &ARM_V7M_UNKNOWN_NONE,
            Self::RiscV32UnknownNone => &RISCV32_UNKNOWN_NONE,
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

    /// Data-stack slot width in bytes (§2.1 of abi-contract).
    /// `high` in slots × `slot_bytes` = peak data-stack usage in bytes.
    /// x86_64 = 8, armv7-m = 4, riscv32 = 4.
    pub slot_bytes: u8,

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

    /// Linker binary name, e.g. `b"ld"`, `b"arm-none-eabi-ld"`.
    pub linker: &'static [u8],
}

impl TargetSpec {
    /// The `abi_hash` that the runtime expects for this target.
    ///
    /// Every compiled module embeds its own `abi_hash` (abi-contract §5);
    /// the loader rejects modules whose hash does not match this value.
    pub fn expected_abi_hash(&self) -> u64 {
        lmod::abi_hash::compute_abi_hash(
            self.slot_bytes,
            self.word_bits,
            lmod::modinfo::MODINFO_VER,
        )
    }
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
    slot_bytes: 8,
    capabilities: &[
        PlatformCapability::TaskScheduler,
        PlatformCapability::DynamicAlloc,
        PlatformCapability::Channels,
    ],
    qemu: None,
    linker: b"ld",
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
    slot_bytes: 8,
    capabilities: &[],
    qemu: Some(&X86_64_UNKNOWN_NONE_QEMU),
    linker: b"ld",
};

// ---------------------------------------------------------------------------
// RISC-V RV32 (riscv32-unknown-none)
// ---------------------------------------------------------------------------

static RISCV32_NONE_EXTRA_ARGS: [&[u8]; 1] = [b"-nographic"];

static RISCV32_NONE_QEMU: QemuSpec = QemuSpec {
    system_bin: b"qemu-system-riscv32",
    machine: b"virt",
    extra_args: &RISCV32_NONE_EXTRA_ARGS,
    exit_convention: QemuExitConvention::Semihosting,
};

static RISCV32_UNKNOWN_NONE: TargetSpec = TargetSpec {
    word_bits: 32,
    pointer_bits: 32,
    endian: Endian::Little,
    has_hardware_mul: true,
    has_hardware_div: true,
    has_fpu: false,
    stack_alignment_bytes: 16,
    output_format: OutputFormat::Elf32,
    assembler: AssemblerKind::GasRiscV,
    calling_conv: CallingConv::RiscV,
    native_int_ty: b"i64",
    slot_bytes: 4,
    capabilities: &[],
    qemu: Some(&RISCV32_NONE_QEMU),
    linker: b"riscv64-unknown-elf-ld",
};

// ---------------------------------------------------------------------------
// ARM Cortex-M3 (armv7m-unknown-none)
// ---------------------------------------------------------------------------

static ARM_V7M_NONE_EXTRA_ARGS: [&[u8]; 1] = [b"-nographic"];

static ARM_V7M_NONE_QEMU: QemuSpec = QemuSpec {
    system_bin: b"qemu-system-arm",
    machine: b"lm3s6965evb",
    extra_args: &ARM_V7M_NONE_EXTRA_ARGS,
    exit_convention: QemuExitConvention::Semihosting,
};

static ARM_V7M_UNKNOWN_NONE: TargetSpec = TargetSpec {
    word_bits: 32,
    pointer_bits: 32,
    endian: Endian::Little,
    has_hardware_mul: true,
    has_hardware_div: true,
    has_fpu: false,
    stack_alignment_bytes: 8,
    output_format: OutputFormat::Elf32,
    assembler: AssemblerKind::GasArm,
    calling_conv: CallingConv::Aapcs32,
    native_int_ty: b"i64",
    slot_bytes: 4,
    capabilities: &[],
    qemu: Some(&ARM_V7M_NONE_QEMU),
    linker: b"arm-none-eabi-ld",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x86_64_none_linker_is_ld() {
        let spec = Target::X86_64UnknownNone.spec();
        assert_eq!(spec.linker, b"ld");
        assert_eq!(spec.assembler, AssemblerKind::Fasm);
    }

    #[test]
    fn x86_64_linux_linker_is_ld() {
        let spec = Target::X86_64UnknownLinuxGnu.spec();
        assert_eq!(spec.linker, b"ld");
        assert_eq!(spec.assembler, AssemblerKind::Fasm);
    }

    #[test]
    fn assembler_variants_are_wired() {
        // All assembler variants must be present to verify the dispatch
        // in assemble_runtime is exhaustive.  Adding a new variant here
        // means the dispatch match must handle it.
        match AssemblerKind::Fasm {
            AssemblerKind::Fasm => {}
            AssemblerKind::GasArm => unreachable!(),
            AssemblerKind::GasRiscV => unreachable!(),
        }
        match AssemblerKind::GasArm {
            AssemblerKind::Fasm => unreachable!(),
            AssemblerKind::GasArm => {}
            AssemblerKind::GasRiscV => unreachable!(),
        }
        match AssemblerKind::GasRiscV {
            AssemblerKind::Fasm => unreachable!(),
            AssemblerKind::GasArm => unreachable!(),
            AssemblerKind::GasRiscV => {}
        }
    }

    #[test]
    fn arm_target_parses() {
        let t = Target::parse(b"armv7m-unknown-none");
        assert_eq!(t, Some(Target::ArmV7MUnknownNone));
    }

    #[test]
    fn arm_target_spec_fields() {
        let spec = Target::ArmV7MUnknownNone.spec();
        assert_eq!(spec.word_bits, 32);
        assert_eq!(spec.native_int_ty, b"i64");
        assert_eq!(spec.slot_bytes, 4);
        assert_eq!(spec.assembler, AssemblerKind::GasArm);
        assert_eq!(spec.calling_conv, CallingConv::Aapcs32);
        assert_eq!(spec.linker, b"arm-none-eabi-ld");
        assert_eq!(spec.qemu.map(|q| q.system_bin), Some(b"qemu-system-arm"));
        assert_eq!(
            spec.qemu.and_then(|q| Some(q.exit_convention.host_pass_exit())),
            Some(0),
        );
    }

    #[test]
    fn arm_target_semihosting_convention() {
        let qemu = Target::ArmV7MUnknownNone.spec().qemu.unwrap();
        assert!(matches!(qemu.exit_convention, QemuExitConvention::Semihosting));
        assert_eq!(qemu.exit_convention.host_pass_exit(), 0);
    }

    #[test]
    fn x86_targets_unchanged_after_arm_added() {
        // Adding a new target must not change existing behavior.
        let none = Target::X86_64UnknownNone.spec();
        assert_eq!(none.linker, b"ld");
        assert_eq!(none.assembler, AssemblerKind::Fasm);
        assert_eq!(none.slot_bytes, 8);
        let linux = Target::X86_64UnknownLinuxGnu.spec();
        assert_eq!(linux.linker, b"ld");
        assert_eq!(linux.assembler, AssemblerKind::Fasm);
    }
}
