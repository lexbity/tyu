import Tyu.Bundles.Bundle

/-! The riscv32-unknown-none bundle instance (PLAN-VERIFY-3 P12.2).

The RAM window mirrors `platforms/riscv32-unknown-none/model/model.toml`
(`[memory] ram = { origin = 0x80000000, length = 0x08000000 }` — the QEMU
virt DRAM). The window numbers are pinned mechanically by the Rust
`bundle_instance_conformance` suite against this instance's `--geometry`
report. -/

namespace Tyu.Bundles

/-- The riscv32-unknown-none instance: DRAM `[0x80000000, 0x87FFFFFF]`
(inclusive). -/
def riscv32 : BundleMem :=
  { ramLo := 0x80000000, ramHi := 0x87FFFFFF, cell := fun _ => Tyu.Mem.IntervalVal.top }

end Tyu.Bundles