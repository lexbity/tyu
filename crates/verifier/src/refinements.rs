//! Device-refinement binding (PLAN-VERIFY-3 P13.1, §Q13/§6.2).
//!
//! A *device refinement* is a named abstract state machine a bundle MAY
//! declare for an MMIO register. Absent a refinement, an MMIO read is the
//! §Q13 nondeterministic width-bounded oracle; under a refinement, a
//! statement that depends on a read of that register is *relativized*: its
//! `StatementContext.refinement` records the refinement name, so a proof
//! against it is meaningful only for a bundle whose model carries that
//! refinement (the `(triple, model_semantics)` relativity of §Q3, extended
//! by the refinement name).
//!
//! This module owns the shared, target-agnostic pieces of that mechanism:
//!
//! - [`Refinement`] — one declared refinement: the register it models (the
//!   trailing register token of an IR `vol_load`/`vol_store` place, see the
//!   matcher), the statement-context refinement id, and the register width;
//! - [`word_refinement`] — resolve the refinement a word's IR is bound to by
//!   scanning its MMIO place tokens. Used identically by the Rust digest
//!   verifier (tyu's E6418 gate) and (via the rendered manifest) the port's
//!   `gen` renderer, so both sides bind the same statement;
//! - [`render_manifest`] / [`parse_manifest`] — the `tyu.refinements/1`
//!   document tyu hands to the port's renderer (the "refinement in context").
//!
//! The manifest schema (model artifact `[refinements]`, P13.1):
//!
//! ```toml
//! [refinements]
//! [[refinements.device]]
//! register   = "UARTFR"         # trailing register token of the IR place
//! refinement = "rp2350.uart-fr" # the §Q13 refinement id (statement context)
//! width      = 32               # register width in bits
//! mask       = 0xF9             # datasheet-transcribed flag/field band
//! mode       = "ro"             # access-mode (closed set, see [`ACCESS_MODES`])
//! ```
//!
//! `mask` is the datasheet-transcribed flag band — the refinement's modeled
//! value set `[0, mask]` (P13.2). It lives in the MANIFEST and the port's
//! bundle instance is pinned against it mechanically (`--level bands`,
//! `bundle_instance_conformance`), so a transcription is a reviewed
//! artifact-pair, not prose (P3 finding). `mode` is the source-level
//! access-mode closure.
//!
//! Written to the renderer as `tyu.refinements/1`:
//!
//! ```json
//! {"schema":"tyu.refinements/1","model":"tyu.model/rp2350/1",
//!  "devices":[{"mask":249,"mode":"ro","refinement":"rp2350.uart-fr",
//!              "register":"UARTFR","width":32}]}
//! ```
//!
//! FR-15 discipline: hand-rolled JSON (no serde), fixed key order. FR-14:
//! no integrity digest lives here (the manifest is context, not evidence —
//! statement hashes bind the claims, and those are SHA-256 in `stmt`).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::codec::{push_i64_json, push_str_json};

/// Schema identifier of the refinement-context document given to renderers.
pub const REFINEMENTS_SCHEMA: &str = "tyu.refinements/1";

/// One declared device refinement (§Q13, P13.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Refinement {
    /// The register the refinement models: the *trailing register token* of
    /// an IR MMIO place (e.g. the `UARTFR` in `uart0.UARTFR` or
    /// `uart.UARTFR`). A register name containing a dot is matched whole-
    /// token; a bare name is matched as a trailing dot-segment, so one
    /// datasheet-transcribed refinement covers the register under any app
    /// const name.
    pub register: String,
    /// The §Q13 refinement id recorded in `StatementContext.refinement` and
    /// carried in the canonical statement context (`{"refinement": "…"}`).
    /// Part of the model identity: part of the bundle's model semantics.
    pub refinement: String,
    /// The register's width in bits (the read domain a refined statement
    /// binds). Closed set {8, 16, 32, 64} — enforced by the tyu lint.
    pub width: u16,
    /// The datasheet-transcribed flag/field **band** the refinement models:
    /// reads answer within `[0, mask]` (P13.2). Lives in the manifest AND in
    /// the port's bundle instance (`Tyu.Bundles`), pinned mechanically
    /// (`--level bands`, `bundle_instance_conformance`) — a datasheet
    /// transcription is a reviewed artifact-pair, not prose.
    pub mask: u32,
    /// The register's access-mode (the closed set [`ACCESS_MODES`]) — the
    /// source-level access semantics the refinement transcribes; the abstract
    /// model expresses it as: `ro` ⇒ the band, write-capable ⇒ the §Q13
    /// width-bounded default, aperture writes inert on reads (§Q13/P13.2).
    pub mode: String,
}

/// The closed set of register access modes the refinement manifest accepts
/// (`ro`, `wo`, `rw`, `w1c`, `w1s`, `rc`) — the source-level access
/// semantics, mirroring the descriptor's register access kinds. Enforced by
/// the tyu lint (E5417) and the manifest parser.
pub const ACCESS_MODES: &[&str] = &["ro", "wo", "rw", "w1c", "w1s", "rc"];

/// Whole-token trailing-segment match: the IR place token `uart0.UARTFR`
/// (or exactly `UARTFR`) matches the declared register `UARTFR`; a
/// dot-bearing declaration (`uart0.UARTFR`) requires exact equality.
fn token_matches(place: &str, register: &str) -> bool {
    if register.is_empty() {
        return false;
    }
    if register.find('.').is_some() {
        return place == register;
    }
    if place == register {
        return true;
    }
    // `place` must end with `.` + register (no alloc::format! — there is no
    // suffix-building allocation below; the slice check is direct).
    let dot = place.rfind('.').map(|i| i + 1).unwrap_or(usize::MAX);
    dot < place.len() && place[dot..] == *register
}

/// The place-token extractor: for one canonical op-text line of the MMIO
/// family, the register token — `vol_load <ty> <place> …` /
/// `vol_store <ty> <place> …` / `vol_load_field <ty> <place> …` /
/// `vol_store_field <ty> <place> …` (the place is token 2), and
/// `addr_of <place> …` / `addr_of_mut <place> …` / `mmio_place <place> …`
/// (the place is token 1). `None` for every other line.
fn place_token(line: &str) -> Option<&str> {
    let t = line.trim();
    let mut tokens = t.split_whitespace();
    let m = tokens.next()?;
    match m {
        "vol_load" | "vol_store" | "vol_load_field" | "vol_store_field" => tokens.nth(1),
        "addr_of" | "addr_of_mut" | "mmio_place" => tokens.next(),
        _ => None,
    }
}

/// Resolve the refinement a word's IR binds to: the FIRST declared
/// refinement whose register matches any MMIO place token of the word's
/// canonical op text. `None` = the word reads no refined register (its
/// read sites stay the §Q13 nondeterministic default).
pub fn word_refinement<'a>(ir: &str, decls: &'a [Refinement]) -> Option<&'a Refinement> {
    if decls.is_empty() {
        return None;
    }
    let mut places: Vec<&str> = Vec::new();
    for line in ir.lines() {
        if let Some(p) = place_token(line) {
            if !places.contains(&p) {
                places.push(p);
            }
        }
    }
    if places.is_empty() {
        return None;
    }
    decls
        .iter()
        .find(|d| places.iter().any(|p| token_matches(p, &d.register)))
}

/// The `tyu.refinements/1` document for the renderer — deterministic, fixed
/// key order (`schema`, `model`, `devices`; device keys `refinement`,
/// `register`, `width` — lexicographic).
pub fn render_manifest(model: &str, decls: &[Refinement], out: &mut Vec<u8>) {
    out.push(b'{');
    out.extend_from_slice(b"\"schema\":");
    push_str_json(out, REFINEMENTS_SCHEMA);
    out.extend_from_slice(b",\"model\":");
    push_str_json(out, model);
    out.extend_from_slice(b",\"devices\":[");
    for (i, d) in decls.iter().enumerate() {
        if i != 0 {
            out.push(b',');
        }
        out.push(b'{');
        // Lexicographic key order: mask, mode, refinement, register, width.
        out.extend_from_slice(b"\"mask\":");
        push_i64_json(out, d.mask as i64);
        out.extend_from_slice(b",\"mode\":");
        push_str_json(out, &d.mode);
        out.extend_from_slice(b",\"refinement\":");
        push_str_json(out, &d.refinement);
        out.extend_from_slice(b",\"register\":");
        push_str_json(out, &d.register);
        out.extend_from_slice(b",\"width\":");
        push_i64_json(out, d.width as i64);
        out.push(b'}');
    }
    out.extend_from_slice(b"]}");
}

/// A minimal hand-rolled reader for the renderer's context document
/// (FR-15), used by the round-trip test and future consumers. `None` on any
/// malformed shape — the caller fails closed.
pub fn parse_manifest(text: &str) -> Option<Vec<Refinement>> {
    let rest = text.strip_prefix("{\"schema\":")?;
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    if &rest[..end] != REFINEMENTS_SCHEMA {
        return None;
    }
    let rest = &rest[end + 1..];
    let (_, rest) = take_string_literal(rest, "model")?;
    let (devices, _) = take_device_array(rest)?;
    let mut out = Vec::with_capacity(devices.len());
    for d in devices {
        // The render emits `{"mask":…,"mode":…,"refinement":…,"register":…,
        // "width":…}` (lexicographic key order) — read in that order.
        let (mask, rest1) = take_int_literal(d, "mask")?;
        let (mode, rest2) = take_string_literal(rest1, "mode")?;
        let (refi, rest3) = take_string_literal(rest2, "refinement")?;
        let (reg, rest4) = take_string_literal(rest3, "register")?;
        let (width, _) = take_int_literal(rest4, "width")?;
        out.push(Refinement {
            register: reg,
            refinement: refi,
            width: width as u16,
            mask: mask as u32,
            mode,
        });
    }
    Some(out)
}

/// Take `"<key>":"<value>"` (an optional leading `,` — the render emits
/// lexicographic key order, the first device field has no comma) and return
/// `(value, remainder-after-the-literal)`.
fn take_string_literal<'a>(rest: &'a str, key: &str) -> Option<(String, &'a str)> {
    let rest = rest.strip_prefix(',').unwrap_or(rest);
    let needle: alloc::string::String = {
        let mut s = alloc::string::String::with_capacity(key.len() + 3);
        s.push('"');
        s.push_str(key);
        s.push('"');
        s.push(':');
        s.push('"');
        s
    };
    let rest = rest.strip_prefix(needle.as_str())?;
    let end = rest.find('"')?;
    Some((rest[..end].to_string(), &rest[end + 1..]))
}

/// Take `"<key>":<int>` (an optional leading `,`; the trailing device
/// field — see [`take_string_literal`]).
fn take_int_literal<'a>(rest: &'a str, key: &str) -> Option<(i64, &'a str)> {
    let rest = rest.strip_prefix(',').unwrap_or(rest);
    let needle: alloc::string::String = {
        let mut s = alloc::string::String::with_capacity(key.len() + 3);
        s.push('"');
        s.push_str(key);
        s.push('"');
        s.push(':');
        s
    };
    let rest = rest.strip_prefix(needle.as_str())?;
    let end = rest
        .find(|c: char| c != '-' && !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let v: i64 = rest[..end].parse().ok()?;
    Some((v, &rest[end..]))
}

/// Take the `,"devices":[{…},{…}]` array and return the per-object closed
/// substrings (from after `{` to the matching `}`).
fn take_device_array(rest: &str) -> Option<(Vec<&str>, &str)> {
    let rest = rest.strip_prefix(",\"devices\":[")?;
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let here = &rest[pos..];
        if let Some(t) = here.strip_prefix(']') {
            return Some((out, t));
        }
        // One device: an optional separating `,` then `{` body `}`. The body
        // is the substring between the braces (no nested `}` in the fixed
        // manifest shape).
        let after_comma = here.strip_prefix(',').map(|_| pos + 1).unwrap_or(pos);
        let body = rest[after_comma..].strip_prefix('{')?;
        let j = body.find('}')?;
        out.push(&body[..j]);
        pos = after_comma + 1 + j + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn decl(register: &str, refinement: &str, width: u16) -> Refinement {
        Refinement {
            register: register.to_string(),
            refinement: refinement.to_string(),
            width,
            mask: 0,
            mode: "ro".to_string(),
        }
    }

    fn decl_full(
        register: &str,
        refinement: &str,
        width: u16,
        mask: u32,
        mode: &str,
    ) -> Refinement {
        Refinement {
            register: register.to_string(),
            refinement: refinement.to_string(),
            width,
            mask,
            mode: mode.to_string(),
        }
    }

    #[test]
    fn token_matches_trailing_segment() {
        // Bare register: whole-token or trailing dot-segment.
        assert!(token_matches("uart0.UARTFR", "UARTFR"));
        assert!(token_matches("uart.UARTFR", "UARTFR"));
        assert!(token_matches("UARTFR", "UARTFR"));
        assert!(!token_matches("UARTFRX", "UARTFR"));
        assert!(!token_matches("MYUARTFR", "UARTFR"));
        // Dot-bearing register: exact equality only.
        assert!(token_matches("uart0.UARTFR", "uart0.UARTFR"));
        assert!(!token_matches("uart.UARTFR", "uart0.UARTFR"));
    }

    #[test]
    fn word_refinement_scans_mmio_lines() {
        let ir = "\
block b0
addr_of uart0.UARTFR aperture=0 offset=0x70018
vol_load u32 uart0.UARTFR atomic=32
local_set 1
ret";
        let decls = [decl("UARTFR", "rp2350.uart-fr", 32)];
        let r = word_refinement(ir, &decls).expect("bound");
        assert_eq!(r.refinement, "rp2350.uart-fr");
        assert_eq!(r.width, 32);
        // A word with no MMIO access binds nothing.
        assert_eq!(
            word_refinement("block b0\nconst_i64 42\nret\n", &decls),
            None
        );
        // A word reading an UNrefined register binds nothing (the §Q13
        // nondeterministic default stays).
        let other = "block b0\nvol_load u32 gpio0.GPIO0_STATUS rw 32 - - - \nret";
        assert_eq!(word_refinement(other, &decls), None);
    }

    #[test]
    fn manifest_round_trips() {
        let mut bytes = Vec::new();
        render_manifest(
            "tyu.model/rp2350/1",
            &[
                decl_full("UARTFR", "rp2350.uart-fr", 32, 0xF9, "ro"),
                decl_full("SPI0.SSPSR", "rp2350.spi-status", 32, 0xFF, "rw"),
            ],
            &mut bytes,
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            text.contains("\"mask\":249") && text.contains("\"mode\":\"ro\""),
            "the datasheet band + access mode must ride the manifest: {text}"
        );
        let got = parse_manifest(&text).expect("round-trip");
        assert_eq!(
            got,
            vec![
                decl_full("UARTFR", "rp2350.uart-fr", 32, 0xF9, "ro"),
                decl_full("SPI0.SSPSR", "rp2350.spi-status", 32, 0xFF, "rw"),
            ]
        );
        // Malformed (wrong schema) parses to None — fail-closed.
        assert_eq!(parse_manifest("{\"schema\":\"tyu.refinements/9\"}"), None);
    }

    #[test]
    fn access_mode_set_is_closed() {
        // P3 finding: the access-mode is a source-level closed set — the six
        // descriptor access kinds. Anything else must be linted as invalid
        // (E5417).
        assert_eq!(ACCESS_MODES, &["ro", "wo", "rw", "w1c", "w1s", "rc"]);
    }

    #[test]
    fn render_is_deterministic() {
        let decls = [decl("UARTFR", "rp2350.uart-fr", 32)];
        let mut a = Vec::new();
        let mut b = Vec::new();
        render_manifest("tyu.model/rp2350/1", &decls, &mut a);
        render_manifest("tyu.model/rp2350/1", &decls, &mut b);
        assert_eq!(a, b);
    }
}
