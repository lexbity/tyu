import Tyu.Bundles.Bundle

/-! The armv7m-unknown-none bundle instance (PLAN-VERIFY-3 P12.2).

The RAM window mirrors `platforms/armv7m-unknown-none/model/model.toml`
(`[memory] ram = { origin = 0x20000000, length = 0x10000 }` — the pack's
`[memory]` SRAM). The window numbers are pinned mechanically by the Rust
`bundle_instance_conformance` suite against this instance's `--geometry`
report. -/

namespace Tyu.Bundles

/-- The armv7m-unknown-none instance: SRAM `[0x20000000, 0x2000FFFF]`
(inclusive). -/
def armv7m : BundleMem :=
  { ramLo := 0x20000000, ramHi := 0x2000FFFF, cell := fun _ => Tyu.Mem.IntervalVal.top }

end Tyu.Bundles