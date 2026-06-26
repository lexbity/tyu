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
    /// All known target triples. Used for exhaustive iteration in tests.
    pub const ALL: [Target; 4] = [
        Target::X86_64UnknownLinuxGnu,
        Target::X86_64UnknownNone,
        Target::ArmV7MUnknownNone,
        Target::RiscV32UnknownNone,
    ];

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

impl CallingConv {
    /// Target-identity discriminant folded into `abi_hash` (abi-contract §5
    /// input #2). Two targets that share `slot_bytes`/`word_bits` but differ in
    /// calling convention (armv7-m vs riscv32) get distinct hashes so a
    /// cross-arch load is rejected. The values are a permanent part of the wire
    /// contract and MUST match `lmod::abi_hash::ARCH_TAG_*` — never renumber.
    pub fn arch_tag(self) -> u8 {
        match self {
            CallingConv::SysV64 => 1,  // ARCH_TAG_X86_64
            CallingConv::Aapcs32 => 2, // ARCH_TAG_ARM
            CallingConv::RiscV => 3,   // ARCH_TAG_RISCV
        }
    }
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

impl PlatformCapability {
    /// Canonical name as used in `manifest.toml` `requires` fields.
    pub fn name(self) -> &'static str {
        match self {
            PlatformCapability::TaskScheduler => "TaskScheduler",
            PlatformCapability::DynamicAlloc => "DynamicAlloc",
            PlatformCapability::Channels => "Channels",
        }
    }
}

// ---------------------------------------------------------------------------
// Feature — image-level build features orthogonal to target triples
// ---------------------------------------------------------------------------

/// Image-level build features selected per `[profile.<name>]` in `tyu.toml`.
///
/// A feature gates both (a) the semantic layer (which language constructs are
/// accepted) and (b) the link step (which runtime units are included).  Axes:
///
/// | Feature          | Language construct              | Runtime unit     |
/// |------------------|--------------------------------|------------------|
/// | `Concurrency`    | `task spawn`                   | `concurrency.asm`|
/// | `ModuleLoading`  | loader/module-load constructs  | `modload.asm`    |
///
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feature {
    Concurrency,
    ModuleLoading,
}

impl core::fmt::Display for Feature {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Feature {
    /// All known features, in declaration order.
    pub const ALL: [Feature; 2] = [Feature::Concurrency, Feature::ModuleLoading];

    /// Canonical string name as used in `tyu.toml` and `--features=<csv>`.
    pub fn as_str(self) -> &'static str {
        match self {
            Feature::Concurrency => "concurrency",
            Feature::ModuleLoading => "module-loading",
        }
    }

    /// Parse a feature name from its string representation.
    /// Returns `None` for an unrecognized string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "concurrency" => Some(Feature::Concurrency),
            "module-loading" => Some(Feature::ModuleLoading),
            _ => None,
        }
    }

    /// The runtime asm unit stem linked when this feature is enabled.
    /// `None` means the feature has no separable runtime unit (it is part
    /// of the mandatory core).
    pub fn runtime_unit(self) -> Option<&'static str> {
        match self {
            Feature::Concurrency => Some("concurrency"),
            Feature::ModuleLoading => Some("modload"),
        }
    }
}

/// A compact bitset of enabled [`Feature`]s.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FeatureSet(u8);

impl FeatureSet {
    /// All features enabled.
    pub fn all() -> Self {
        let mut s = Self(0);
        for f in &Feature::ALL {
            s = s.with(*f);
        }
        s
    }

    /// No features enabled.
    pub fn empty() -> Self {
        Self(0)
    }

    /// Return a new set with `f` added.
    pub fn with(self, f: Feature) -> Self {
        Self(self.0 | (1 << (f as u8)))
    }

    /// Does this set contain `f`?
    pub fn contains(self, f: Feature) -> bool {
        (self.0 & (1 << (f as u8))) != 0
    }

    /// Iterate over enabled features in declaration order.
    pub fn iter(self) -> impl Iterator<Item = Feature> {
        Feature::ALL
            .iter()
            .copied()
            .filter(move |f| self.contains(*f))
    }

    /// Write enabled feature names into `out` (up to its length) and return
    /// the number written.  Typical usage:
    /// ```ignore
    /// let mut buf = [""; 2];
    /// let n = set.write_flags(&mut buf);
    /// let csv = buf[..n].join(",");
    /// ```
    pub fn write_flags(self, out: &mut [&'static str]) -> usize {
        let mut i = 0;
        for f in self.iter() {
            if i < out.len() {
                out[i] = f.as_str();
                i += 1;
            }
        }
        i
    }
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

/// A RAM-backed or device-backed scratch address that a QEMU machine answers.
///
/// Used by MMIO fixtures to perform a volatile load/store round-trip. The
/// address must be mapped and accessible without triggering a fault.
#[derive(Clone, Copy, Debug)]
pub struct MmioScratch {
    /// Physical address of the scratch region.
    pub addr: u64,
    /// Whether the address is backed by RAM or by a device register.
    pub backed: ScratchBacking,
}

/// How the scratch address is backed in the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScratchBacking {
    Ram,
    Device,
}

/// An interrupt source available on a QEMU machine.
///
/// When `Some`, the machine can deliver timer interrupts that the kernel can
/// handle via the `@interrupt` handler mechanism.  The specific variant names
/// the interrupt controller and delivery mechanism.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterruptSource {
    /// ARM Cortex-M SysTick timer at `0xE000E010`.
    CortexMSysTick,
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
    /// A RAM-backed or device-backed address for MMIO smoke tests.
    /// `None` means the machine has no address the MMIO fixture can safely
    /// use — the `mmio` coverage axis is not required for such targets.
    pub mmio_scratch: Option<MmioScratch>,
    /// An interrupt source available on this machine.
    /// `None` means the machine has no wired interrupt delivery — the
    /// `interrupt` coverage axis is not required for such targets.
    pub interrupt_source: Option<InterruptSource>,
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
            self.calling_conv.arch_tag(),
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
    // q35 with -m 32M maps RAM from 0x0; 0x100000 is safely in RAM past the
    // legacy BIOS/VGA region (0xA0000-0xFFFFF).
    mmio_scratch: Some(MmioScratch {
        addr: 0x100000,
        backed: ScratchBacking::Ram,
    }),
    // x86 q35 has no wired interrupt delivery in our setup (no i8259/PIC
    // programming); the interrupt axis is not required.
    interrupt_source: None,
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
    qemu: Some(&X86_64_UNKNOWN_NONE_QEMU),
    linker: b"ld",
};

// ---------------------------------------------------------------------------
// RISC-V RV32 (riscv32-unknown-none)
// ---------------------------------------------------------------------------

// `-bios none`: the `virt` machine loads OpenSBI at 0x80000000 by default,
// which collides with our kernel (also linked at 0x80000000).  Booting bare
// metal requires suppressing the default firmware so the CPU resets straight
// into our image at the start of DRAM.
static RISCV32_NONE_EXTRA_ARGS: [&[u8]; 3] = [b"-bios", b"none", b"-nographic"];

static RISCV32_NONE_QEMU: QemuSpec = QemuSpec {
    system_bin: b"qemu-system-riscv32",
    machine: b"virt",
    extra_args: &RISCV32_NONE_EXTRA_ARGS,
    exit_convention: QemuExitConvention::Semihosting,
    // RISC-V virt machine: DRAM starts at 0x80000000; first few pages are safe.
    mmio_scratch: Some(MmioScratch {
        addr: 0x80000000,
        backed: ScratchBacking::Ram,
    }),
    // virt has no wired interrupt delivery via CLINT in our runtime yet
    // (no SiFive CLINT driver); interrupt axis is not required until
    // follow-on wires CLINT timer delivery.
    interrupt_source: None,
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
    qemu: Some(&RISCV32_NONE_QEMU),
    linker: b"riscv32-elf-ld",
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
    // lm3s6965evb: 256 KB SRAM at 0x20000000-0x2003FFFF.
    mmio_scratch: Some(MmioScratch {
        addr: 0x20000000,
        backed: ScratchBacking::Ram,
    }),
    // lm3s6965evb has a Cortex-M3 SysTick timer at 0xE000E010. The
    // isr_lock_atomicity fixture implements a @interrupt(SysTick) handler.
    interrupt_source: Some(InterruptSource::CortexMSysTick),
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
        assert_eq!(
            spec.qemu.map(|q| q.system_bin),
            Some(&b"qemu-system-arm"[..])
        );
        assert_eq!(
            spec.qemu
                .and_then(|q| Some(q.exit_convention.host_pass_exit())),
            Some(0),
        );
    }

    #[test]
    fn arm_target_semihosting_convention() {
        let qemu = Target::ArmV7MUnknownNone.spec().qemu.unwrap();
        assert!(matches!(
            qemu.exit_convention,
            QemuExitConvention::Semihosting
        ));
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

    // -----------------------------------------------------------------------
    // Feature / FeatureSet
    // -----------------------------------------------------------------------

    #[test]
    fn feature_parse_roundtrip() {
        for f in &Feature::ALL {
            let s = f.as_str();
            let parsed = Feature::parse(s).expect("parse own as_str");
            assert_eq!(parsed, *f);
        }
    }

    #[test]
    fn feature_parse_unknown_is_none() {
        assert!(Feature::parse("bogus").is_none());
        assert!(Feature::parse("concurrency ").is_none()); // trailing space
    }

    #[test]
    fn feature_set_all() {
        let set = FeatureSet::all();
        assert!(set.contains(Feature::Concurrency));
        assert!(set.contains(Feature::ModuleLoading));
    }

    #[test]
    fn feature_set_empty() {
        let set = FeatureSet::empty();
        assert!(!set.contains(Feature::Concurrency));
        assert!(!set.contains(Feature::ModuleLoading));
    }

    #[test]
    fn feature_set_with_and_iter() {
        let set = FeatureSet::empty().with(Feature::Concurrency);
        assert!(set.contains(Feature::Concurrency));
        assert!(!set.contains(Feature::ModuleLoading));
        let mut count = 0u32;
        for _f in set.iter() {
            count += 1;
        }
        assert_eq!(count, 1);
    }

    #[test]
    fn feature_set_write_flags() {
        let set = FeatureSet::all();
        let mut buf = [""; 4];
        let n = set.write_flags(&mut buf);
        assert_eq!(n, 2);
        assert!(buf[..n].contains(&"concurrency"));
        assert!(buf[..n].contains(&"module-loading"));
    }

    #[test]
    fn feature_runtime_unit() {
        assert_eq!(Feature::Concurrency.runtime_unit(), Some("concurrency"));
        assert_eq!(Feature::ModuleLoading.runtime_unit(), Some("modload"));
    }
}
