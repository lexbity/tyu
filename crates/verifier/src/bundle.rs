//! The bundle model-artifact geometry surface (PLAN-VERIFY-3 P12.2;
//! cleanup item 3 — the ONE parser for the `[memory] ram` window).
//!
//! `model/model.toml`'s `[memory] ram = { origin = …, length = … }` window
//! is the single geometry source the bundle-instance surface consumes: the
//! Rust `ApertureMem` instances and the `tyu.vec/1` corpus `"ram"` header
//! (the `bundle_instance_conformance` suite), the Lean `Tyu.Bundles`
//! fixtures (pinned via `--level bundles`), and the tyu-side
//! `model_artifact_ram` accessor.
//!
//! The reader is hand-rolled over the fixed, committed shape (this crate is
//! `no_std` + alloc and owns the evidence-surface readers; no TOML
//! dependency). The window is **half-open**: `(origin, origin + length)`;
//! each consumer derives its own inclusive end where needed (the corpus
//! header and the Lean fixtures use `origin + length − 1`).

/// The committed `ram = { … origin = … length = … }` line prefix (within a
/// `[memory]` section). The scanner only ever consults this line, so a stray
/// `origin`/`length` token in the pack docs cannot influence the result.
const RAM_LINE_START: &str = "ram = {";

/// Take the hex value after a `= `-separated token at byte offset `idx`.
/// The committed shape is a `0x`-prefixed literal; a token without the
/// prefix is malformed (`None` — a bare word like `banana` must not be
/// half-parsed as `0xba`).
fn take_hex_after(line: &str, token: &str, idx: usize) -> Option<u64> {
    let rest = line[idx + token.len()..].trim_start();
    // `rest` begins at `=` (the committed shape is `tok = 0x…`); skip `=`
    // and whitespace, require the `0x` prefix, then take the hex word.
    let after_eq = rest.trim_start_matches('=').trim();
    let v = after_eq
        .strip_prefix("0x")
        .or_else(|| after_eq.strip_prefix("0X"))?;
    let end = v.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(v.len());
    u64::from_str_radix(&v[..end], 16).ok()
}

/// Parse the `[memory] ram` window `(origin, origin + length)` (half-open)
/// from the committed model-artifact text. `None` when the fixed shape is
/// absent or malformed — the evidence chain has no geometry then and the
/// caller fails closed.
pub fn model_memory_ram(text: &str) -> Option<(u64, u64)> {
    let mut origin: Option<u64> = None;
    let mut length: Option<u64> = None;
    for line in text.lines() {
        let t = line.trim();
        if !t.starts_with(RAM_LINE_START) {
            continue;
        }
        for token in ["origin", "length"] {
            if let Some(idx) = t.find(token) {
                let value = take_hex_after(t, token, idx)?;
                if token == "origin" {
                    origin = Some(value);
                } else {
                    length = Some(value);
                }
            }
        }
    }
    Some((origin?, origin? + length?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const X86: &str = "\
[model]
id = \"tyu.model/x86_64-unknown-none/1\"

[memory]
ram = { name = \"RAM\", origin = 0x100000, length = 0x100000 }

[refinements]
";
    const ARM: &str = "\
[memory]
ram = { name = \"SRAM\", origin = 0x20000000, length = 0x10000 }
";
    const RV: &str = "\
[memory]
ram = { name = \"DRAM\", origin = 0x80000000, length = 0x08000000 }
";

    /// The committed in-tree windows parse to the exact half-open regions the
    /// evidence chain keys on (these numbers equal the Lean `Tyu.Bundles`
    /// instances' `hi + 1`, pinned by `bundle_geometry_matches_lean_instance`).
    #[test]
    fn committed_windows_parse() {
        assert_eq!(model_memory_ram(X86), Some((0x100000, 0x100000 + 0x100000)));
        assert_eq!(
            model_memory_ram(ARM),
            Some((0x2000_0000, 0x2000_0000 + 0x1_0000))
        );
        assert_eq!(
            model_memory_ram(RV),
            Some((0x8000_0000, 0x8000_0000 + 0x0800_0000))
        );
    }

    /// Absence is `None` (fail-closed — a stray doc token cannot invent a
    /// window).
    #[test]
    fn absent_or_malformed_is_none() {
        assert_eq!(model_memory_ram("# no ram here\norigin = 0x10\n"), None);
        assert_eq!(model_memory_ram("[model]\nid = \"x\"\n"), None);
        // A `ram = {` line with a non-hex origin is malformed.
        assert_eq!(model_memory_ram("ram = { origin = banana }\n"), None);
    }
}
