//! Coverage-axis taxonomy for qualification reporting.

/// A behavior class exercised by one or more execution fixtures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoverageAxis {
    Arith,
    Stack,
    Controlflow,
    CallAbi,
    MemWidth,
    Ptr,
    Locals,
    Trap,
    DeepStack,
    Mmio,
    Interrupt,
    Concurrency,
}

impl CoverageAxis {
    /// Canonical axis ordering for stable reports and snapshots.
    pub const ALL: [CoverageAxis; 12] = [
        CoverageAxis::Arith,
        CoverageAxis::Stack,
        CoverageAxis::Controlflow,
        CoverageAxis::CallAbi,
        CoverageAxis::MemWidth,
        CoverageAxis::Ptr,
        CoverageAxis::Locals,
        CoverageAxis::Trap,
        CoverageAxis::DeepStack,
        CoverageAxis::Mmio,
        CoverageAxis::Interrupt,
        CoverageAxis::Concurrency,
    ];

    pub fn is_core(self) -> bool {
        matches!(self.gate(), AxisGate::Core)
    }

    /// How this axis becomes required for a selection.
    ///
    /// QEMU gates are represented by the machine fact they need, rather than a
    /// function over `QemuSpec`, so this crate stays independent from
    /// `codegen-core` and remains usable by all report consumers.
    pub fn gate(self) -> AxisGate {
        match self {
            CoverageAxis::Concurrency => AxisGate::RuntimeService(&["Channels", "TaskScheduler"]),
            CoverageAxis::Mmio => AxisGate::Qemu(QemuAxisGate::MmioScratch),
            CoverageAxis::Interrupt => AxisGate::Qemu(QemuAxisGate::InterruptSource),
            _ => AxisGate::Core,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "arith" => CoverageAxis::Arith,
            "stack" => CoverageAxis::Stack,
            "controlflow" => CoverageAxis::Controlflow,
            "call-abi" => CoverageAxis::CallAbi,
            "mem-width" => CoverageAxis::MemWidth,
            "ptr" => CoverageAxis::Ptr,
            "locals" => CoverageAxis::Locals,
            "trap" => CoverageAxis::Trap,
            "deep-stack" => CoverageAxis::DeepStack,
            "mmio" => CoverageAxis::Mmio,
            "interrupt" => CoverageAxis::Interrupt,
            "concurrency" => CoverageAxis::Concurrency,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CoverageAxis::Arith => "arith",
            CoverageAxis::Stack => "stack",
            CoverageAxis::Controlflow => "controlflow",
            CoverageAxis::CallAbi => "call-abi",
            CoverageAxis::MemWidth => "mem-width",
            CoverageAxis::Ptr => "ptr",
            CoverageAxis::Locals => "locals",
            CoverageAxis::Trap => "trap",
            CoverageAxis::DeepStack => "deep-stack",
            CoverageAxis::Mmio => "mmio",
            CoverageAxis::Interrupt => "interrupt",
            CoverageAxis::Concurrency => "concurrency",
        }
    }
}

/// Where an axis's required verdict comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisGate {
    Core,
    RuntimeService(&'static [&'static str]),
    Qemu(QemuAxisGate),
}

/// QEMU-machine facts that can require a platform axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QemuAxisGate {
    MmioScratch,
    InterruptSource,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_roundtrip() {
        for axis in CoverageAxis::ALL {
            assert_eq!(CoverageAxis::parse(axis.as_str()), Some(axis));
        }
        assert_eq!(CoverageAxis::parse("callabi"), None);
        assert_eq!(CoverageAxis::parse("memwidth"), None);
    }

    #[test]
    fn core_partition_is_expected_size() {
        let core = CoverageAxis::ALL
            .iter()
            .filter(|axis| axis.is_core())
            .count();
        assert_eq!(core, 9);
        assert_eq!(CoverageAxis::ALL.len() - core, 3);
    }

    #[test]
    fn platform_axis_gates_name_real_sources() {
        assert_eq!(
            CoverageAxis::Concurrency.gate(),
            AxisGate::RuntimeService(&["Channels", "TaskScheduler"])
        );
        assert_eq!(
            CoverageAxis::Mmio.gate(),
            AxisGate::Qemu(QemuAxisGate::MmioScratch)
        );
        assert_eq!(
            CoverageAxis::Interrupt.gate(),
            AxisGate::Qemu(QemuAxisGate::InterruptSource)
        );
    }
}
