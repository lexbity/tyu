import Tyu.Bundles.Bundle

/-! The rp2350 board bundle instance (PLAN-VERIFY-3 P13.2).

The RAM window mirrors `platforms/rp2350/model/model.toml`
(`[memory] ram = { origin = 0x20000000, length = 0x10000 }` — the first
64 KiB of the 520 KiB SRAM region, DS2 Tables 11-12). The window numbers
are pinned mechanically: the Rust `bundle_instance_conformance` suite reads
the model artifact and compares it against this instance's `--level
bundles` report.

This bundle carries the one datasheet-transcribed device refinement
declared in the model artifact's `[refinements]` manifest: the UART0 flag
register `UARTFR` (PL011, DS2 12.1). Its read answers within the ten
modeled flag bits — the `uartFrBand` mask — which is the band a refined
statement may bind against (`Tyu.Sound.TD.uartfr_band_domain` + the T-D
completion laws). -/

namespace Tyu.Bundles

/-- The PL011-family UARTFR (DS2 §12.1 / the UART0 descriptor's `UARTFR` row
at 0x018) flag band: the SIX modeled flag bits the RP2350's UARTFR defines
(mask 0xF9) — NOT the full PL011 layout (bits 3-11, 0xFF8). The band is the
datasheet transcription: a refined read of `UARTFR` answers within
`[0, 0xF9]` — the union over every flag combination. Pinned against
`model/model.toml [[refinements.device]] mask = 0xF9` mechanically
(`--level bands`, `bundle_instance_conformance`), reviewed like T-F2
(`REVIEW.md` §2). -/
def uartFrBand : Int := 0xF9

/-- The UARTFR access-mode: read-only (the descriptor's `access = "ro"`
row). The abstract model's ro semantics: reads answer the band (the
datasheet stable-flag value set, [`Tyu.Sound.TD.ro_read_answers_band`]). -/
def uartFrMode : Tyu.Bundles.AccessMode := Tyu.Bundles.AccessMode.ro

/-- The rp2350 instance: the SRAM window `[0x20000000, 0x2000FFFF]`
(inclusive) with the §Q13 unmodeled default everywhere else. -/
def rp2350 : BundleMem :=
  { ramLo := 0x20000000, ramHi := 0x2000FFFF, cell := fun _ => Tyu.Mem.IntervalVal.top }

end Tyu.Bundles