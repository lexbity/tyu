//! The `verify_manifest` record encoder (PLAN-VERIFY-3 §6.5, P11).
//!
//! Builds the wire record the loader validates (`lmod::verify_manifest`):
//! the canonical obligations region (ids sorted — the loader *verifies* the
//! order) plus a SHA-256 digest over exactly those bytes. The record is
//! appended to the modinfo payload at pack time (`pack_with_verify_manifest`).
//!
//! The CLI path accepts a `tyu.vm/1` JSON summary (hand-rolled reader — no
//! JSON crate, per FR-15).

use sha2::{Digest, Sha256};

/// One obligation entry.
#[derive(Clone, Debug)]
pub struct ObligationSpec {
    pub id: String,
    pub id_hash: u64,
    pub status: u8,
    pub trust: u8,
    pub statement_hash: [u8; 32],
}

/// The manifest summary.
#[derive(Clone, Debug)]
pub struct VerifyManifestSpec {
    pub semantics: String,
    pub stmt: String,
    pub target: String,
    pub model: String,
    pub policy: u8,
    pub certifier_class: u8,
    pub certifier_name: String,
    pub certifier_recognition: String,
    pub candidate_ratio: u16,
    pub counts: [u32; 4],
    pub obligations: Vec<ObligationSpec>,
}

fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_len_bytes(out: &mut Vec<u8>, b: &[u8]) -> Result<(), String> {
    if b.len() > u16::MAX as usize {
        return Err("verify_manifest: field exceeds 64 KiB".to_string());
    }
    push_u16(out, b.len() as u16);
    out.extend_from_slice(b);
    Ok(())
}

/// Encode the record (payload + size + tag). The caller appends it to the
/// modinfo payload at pack time.
///
/// The obligations are **canonicalized here**: sorted by id (bytewise
/// non-decreasing — the loader's scan rejects a decrease), capped, and the
/// digest is SHA-256 over the region as encoded.
pub fn encode_verify_manifest(spec: &VerifyManifestSpec) -> Result<Vec<u8>, String> {
    // Caps (mirror the loader's scan caps).
    if spec.policy > lmod::verify_manifest::VM_POLICY_PROVEN
        || spec.certifier_class > lmod::verify_manifest::VM_CERTIFIER_PORT
    {
        return Err("verify_manifest: policy/certifier class out of the closed set".to_string());
    }
    if spec.obligations.len() > lmod::verify_manifest::VM_MAX_COUNT as usize {
        return Err("verify_manifest: obligation count exceeds the cap".to_string());
    }

    let mut ob = spec.obligations.clone();
    ob.sort_by(|a, b| a.id.as_bytes().cmp(b.id.as_bytes()));

    let mut region: Vec<u8> = Vec::new();
    for o in ob.iter() {
        if o.id.len() > lmod::verify_manifest::VM_MAX_ID_LEN {
            return Err("verify_manifest: obligation id exceeds 512 B".to_string());
        }
        if o.status > lmod::verify_manifest::VM_STATUS_ASSUMED
            || o.trust > lmod::verify_manifest::VM_TRUST_PROOF
        {
            return Err(
                "verify_manifest: obligation status/trust out of the closed set".to_string(),
            );
        }
        push_len_bytes(&mut region, o.id.as_bytes())?;
        push_u64(&mut region, o.id_hash);
        region.push(o.status);
        region.push(o.trust);
        region.extend_from_slice(&o.statement_hash);
    }

    let digest: [u8; 32] = Sha256::digest(&region).into();

    let mut payload: Vec<u8> = Vec::new();
    push_len_bytes(&mut payload, spec.semantics.as_bytes())?;
    push_len_bytes(&mut payload, spec.stmt.as_bytes())?;
    push_len_bytes(&mut payload, spec.target.as_bytes())?;
    push_len_bytes(&mut payload, spec.model.as_bytes())?;
    payload.push(spec.policy);
    payload.push(spec.certifier_class);
    push_len_bytes(&mut payload, spec.certifier_name.as_bytes())?;
    push_len_bytes(&mut payload, spec.certifier_recognition.as_bytes())?;
    push_u16(&mut payload, spec.candidate_ratio);
    push_u32(&mut payload, ob.len() as u32);
    push_u32(&mut payload, spec.counts[0]);
    push_u32(&mut payload, spec.counts[1]);
    push_u32(&mut payload, spec.counts[2]);
    push_u32(&mut payload, spec.counts[3]);
    payload.extend_from_slice(&region);
    payload.extend_from_slice(&digest);

    if payload.len() as u32 > lmod::verify_manifest::VM_MAX_RECORD {
        return Err("verify_manifest: record exceeds the 4 MiB cap".to_string());
    }

    let mut out = payload;
    let size = out.len() as u32;
    push_u32(&mut out, size);
    push_u32(&mut out, lmod::verify_manifest::VM_TAG);
    Ok(out)
}

// ---------------------------------------------------------------------------
// The `tyu.vm/1` CLI summary reader (hand-rolled JSON, FR-15).
// ---------------------------------------------------------------------------

/// Parse a hexadecimal SHA-256 digest.
fn parse_hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let hi = (chunk[0] as char).to_digit(16)? as u8;
        let lo = (chunk[1] as char).to_digit(16)? as u8;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

/// The single "read a `tyu.vm/1` JSON summary → wire record" entry point —
/// the one path build (manifest_record), deploy (static pack), and the
/// `lmod-pack` CLI all use, so the summary reader never drifts from the
/// encoder.
pub fn encode_from_json_text(json: &str) -> Result<Vec<u8>, String> {
    let spec = verify_manifest_from_json(json)?;
    encode_verify_manifest(&spec)
}

/// A tiny JSON parser over the `tyu.vm/1` wire form; returns the object's
/// key → value pairs for string/number/bool leaves (arrays are joined for
/// the obligations list below). Producers are the build's derived summaries
/// (`tyu::vm_summary`) and the port tooling; this parser serves the CLI
/// path (`lmod-pack --verify-manifest`).
///
/// The summary shape:
/// ```json
/// { "schema": "tyu.vm/1", "semantics": "...", "stmt": "...", "target": "...",
///   "model": "...", "policy": "...", "certifier": { "class": "port",
///   "name": "...", "recognition": "..." }, "candidate_ratio": 0,
///   "counts": {"proof": 0, "checked": 0, "assumed": 0, "open": 0},
///   "obligations": [ {"id": "...", "id_hash": 0, "status": "discharged",
///   "trust": "proof", "statement_hash": "<64 hex>"} ] }
/// ```
pub fn verify_manifest_from_json(json: &str) -> Result<VerifyManifestSpec, String> {
    let trimmed = json.trim();
    if !trimmed.starts_with('{') {
        return Err("tyu.vm/1: expected a JSON object".to_string());
    }
    let mut spec = VerifyManifestSpec {
        semantics: String::new(),
        stmt: String::new(),
        target: String::new(),
        model: String::new(),
        policy: 0,
        certifier_class: lmod::verify_manifest::VM_CERTIFIER_NONE,
        certifier_name: String::new(),
        certifier_recognition: String::new(),
        candidate_ratio: 0,
        counts: [0; 4],
        obligations: Vec::new(),
    };
    let obj = parse_object(trimmed)?;
    let mut seen_schema = false;
    let mut counts_obj: Option<Vec<(String, String)>> = None;
    let mut certifier_obj: Option<Vec<(String, String)>> = None;
    for (k, v) in obj {
        match k.as_str() {
            "schema" => {
                if v != "tyu.vm/1" {
                    return Err("tyu.vm/1: unknown schema".to_string());
                }
                seen_schema = true;
            }
            "semantics" => spec.semantics = v,
            "stmt" => spec.stmt = v,
            "target" => spec.target = v,
            "model" => spec.model = v,
            "policy" => {
                spec.policy = match v.as_str() {
                    "open-ok" => lmod::verify_manifest::VM_POLICY_OPEN_OK,
                    "no-open" => lmod::verify_manifest::VM_POLICY_NO_OPEN,
                    "no-open-no-assumptions" => {
                        lmod::verify_manifest::VM_POLICY_NO_OPEN_NO_ASSUMPTIONS
                    }
                    "proven" => lmod::verify_manifest::VM_POLICY_PROVEN,
                    _ => return Err("tyu.vm/1: unknown policy".to_string()),
                }
            }
            "certifier" => certifier_obj = parse_object(&v).ok(),
            "candidate_ratio" => {
                spec.candidate_ratio = v.parse().map_err(|_| "tyu.vm/1: bad candidate_ratio")?
            }
            "counts" => counts_obj = parse_object(&v).ok(),
            "obligations" => spec.obligations = parse_obligations(&v)?,
            _ => {}
        }
    }
    if !seen_schema {
        return Err("tyu.vm/1: no schema field".to_string());
    }
    if let Some(co) = counts_obj {
        let mut counts = [0u32; 4];
        for (k, v) in co {
            let n: u32 = v.parse().map_err(|_| "tyu.vm/1: bad count")?;
            match k.as_str() {
                "proof" => counts[0] = n,
                "checked" => counts[1] = n,
                "assumed" => counts[2] = n,
                "open" => counts[3] = n,
                _ => {}
            }
        }
        spec.counts = counts;
    }
    if let Some(ce) = certifier_obj {
        for (k, v) in ce {
            match k.as_str() {
                "class" => {
                    spec.certifier_class = match v.as_str() {
                        "port" => lmod::verify_manifest::VM_CERTIFIER_PORT,
                        "none" => lmod::verify_manifest::VM_CERTIFIER_NONE,
                        _ => return Err("tyu.vm/1: unknown certifier class".to_string()),
                    }
                }
                "name" => spec.certifier_name = v,
                "recognition" => spec.certifier_recognition = v,
                _ => {}
            }
        }
    }
    Ok(spec)
}

fn parse_object(s: &str) -> Result<Vec<(String, String)>, String> {
    let s = s.trim();
    if !s.starts_with('{') {
        return Err("expected '{'".to_string());
    }
    // The value span ends at the matching close brace (respecting nesting),
    // then key/value pairs are recovered by scanning top-level commas.
    let (inner, _) = split_balanced(s, '{', '}')?;
    let mut out = Vec::new();
    for part in split_top_level(inner, ',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (k, v) = parse_pair(part)?;
        out.push((k, v));
    }
    Ok(out)
}

/// Split a string on `sep` at nesting depth 0 (ignores separators inside
/// `{...}`, `[...]`, and `"..."`).
fn split_top_level(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '{' | '[' if !in_str => depth += 1,
            '}' | ']' if !in_str => depth -= 1,
            c if c == sep && !in_str && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Locate a `{...}`/`[...]` value starting at `s[open_idx]` and return
/// `(inner, close_idx)`.
fn split_balanced(s: &str, open: char, close: char) -> Result<(&str, usize), String> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Ok((&s[1..i], i + 1));
            }
        } else if c == '"' {
            // Skip string contents (cannot contain braces unescaped in our
            // summary files).
            let _ = c;
        }
    }
    Err("unbalanced braces".to_string())
}

fn parse_pair(pair: &str) -> Result<(String, String), String> {
    let pair = pair.trim();
    let colon = pair.find(':').ok_or("expected ':'")?;
    let k = unquote(pair[..colon].trim());
    let raw = pair[colon + 1..].trim();
    let val = if raw.starts_with('{') {
        let (inner, close) = split_balanced(raw, '{', '}')?;
        // Preserve the object text for nested parsing (certifier/counts).
        let _ = inner;
        raw[..close].to_string()
    } else if raw.starts_with('[') {
        let (_, close) = split_balanced(raw, '[', ']')?;
        raw[..close].to_string()
    } else if raw.starts_with('"') {
        unquote(raw)
    } else {
        // number / bare: to the comma
        raw.split(',').next().unwrap_or("").trim().to_string()
    };
    Ok((k, val))
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn parse_obligations(s: &str) -> Result<Vec<ObligationSpec>, String> {
    let s = s.trim();
    if !s.starts_with('[') {
        return Err("expected '[' for obligations".to_string());
    }
    let (inner, _) = split_balanced(s, '[', ']')?;
    let mut out = Vec::new();
    for part in split_top_level(inner, ',') {
        if part.trim().is_empty() {
            continue;
        }
        out.push(parse_obligation_obj(part)?);
    }
    Ok(out)
}

fn parse_obligation_obj(s: &str) -> Result<ObligationSpec, String> {
    let pairs = parse_object(s)?;
    let mut id = String::new();
    let mut id_hash = 0u64;
    let mut status = lmod::verify_manifest::VM_STATUS_OPEN;
    let mut trust = lmod::verify_manifest::VM_TRUST_OPEN;
    let mut statement_hash = [0u8; 32];
    for (k, v) in pairs {
        match k.as_str() {
            "id" => id = v,
            "id_hash" => id_hash = v.parse().map_err(|_| "bad id_hash")?,
            "status" => {
                status = match v.as_str() {
                    "open" => lmod::verify_manifest::VM_STATUS_OPEN,
                    "discharged" => lmod::verify_manifest::VM_STATUS_DISCHARGED,
                    "assumed" => lmod::verify_manifest::VM_STATUS_ASSUMED,
                    _ => return Err("unknown status".to_string()),
                }
            }
            "trust" => {
                trust = match v.as_str() {
                    "open" => lmod::verify_manifest::VM_TRUST_OPEN,
                    "assumed" => lmod::verify_manifest::VM_TRUST_ASSUMED,
                    "checked" => lmod::verify_manifest::VM_TRUST_CHECKED,
                    "proof" => lmod::verify_manifest::VM_TRUST_PROOF,
                    _ => return Err("unknown trust".to_string()),
                }
            }
            "statement_hash" => statement_hash = parse_hex32(&v).ok_or("bad statement_hash")?,
            _ => {}
        }
    }
    Ok(ObligationSpec {
        id,
        id_hash,
        status,
        trust,
        statement_hash,
    })
}
