import Tyu.Bundles.Bundle

/-! The x86_64-unknown-none bundle instance (PLAN-VERIFY-3 P12.2).

The RAM window mirrors `platforms/x86_64-unknown-none/model/model.toml`
(`[memory] ram = { origin = 0x100000, length = 0x100000 }` — the q35 1 MiB
multiboot load region). The window numbers are pinned mechanically: the
Rust `bundle_instance_conformance` suite reads the model artifact and
compares it against this instance's `--geometry` report. -/

namespace Tyu.Bundles

/-- The x86_64-unknown-none instance: whole i64 domain above the modeled
window and unmodeled by default; loads/stores honor
`[0x100000, 0x1FFFFF]` (inclusive). -/
def x86_64 : BundleMem :=
  { ramLo := 0x100000, ramHi := 0x1FFFFF, cell := fun _ => Tyu.Mem.IntervalVal.top }

end Tyu.Bundles