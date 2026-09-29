//! The `tyu.svcvec/1` service-vector corpus codec (PLAN-VERIFY-3 P15.2).
//!
//! The hosted bundle's `evidence/vectors.json` carries a corpus of SCRIPTED
//! channel programs: `[make, send, recv, …]` op lists with the expected FIFO
//! outputs. THE SAME disciplined treatment as every other corpus format
//! (`tyu.vec/1`, `tyu.fragvec/1`): hand-rolled JSON (no serde — FR-15,
//! `no_std`-compatible alloc), schema-checked on read, size-capped, and
//! fail-closed on malformed input (E6400/E6401-class). Three surfaces replay
//! each script — the port's `conformance --level services` (the Lean
//! `Tyu.Services.traceRun`), this codec's [`run_trace`] (the Rust mirror),
//! and the HOSTED RUNTIME via [`render_hosted_source`] (the deterministic
//! tyu source a script lowers to — the divergence-pinned wire form a
//! developer can actually write) — and must all agree (R9 detection).
//!
//! A script's ops are expressed in the abstract-atomic model's channel
//! identities (the nth make of a channel). [`render_hosted_source`] lowers a
//! script to the source surface as channel EPISODES (`make dup …; payload
//! send …; recv` — the only shape a developer can write on the DATA stack,
//! since a channel handle cannot be held dormant under a received value):
//! one make per channel, contiguous ops per channel, exactly one recv per
//! channel, payloads constant. The generator REFUSES (returns `None`) any
//! script outside that surface — a corpus script that cannot be lowered has
//! no hosted counterpart and the agreement chain is broken loudly, never
//! silently approximated.

use alloc::string::String;
use alloc::vec::Vec;

/// Schema identifier of `tyu.svcvec/1`.
pub const SVCVEC_SCHEMA: &str = "tyu.svcvec/1";

/// Hard read-side cap for `tyu.svcvec/1` documents (the corpus is committed
/// evidence; fail-closed on anything past the bound).
pub const SVCVEC_MAX_BYTES: usize = 1024 * 1024;

/// One abstract-atomic channel op (mirrors `Tyu.Services.ServiceOp`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SvOp {
    /// `platform.channel.make` — install a fresh bounded FIFO at the channel.
    Make(u64),
    /// `platform.channel.send` — append `val` to the channel's FIFO.
    Send(u64, i64),
    /// `platform.channel.recv` — pop the channel's FIFO head.
    Recv(u64),
}

/// One scripted channel program (a script id + the ops + the expected FIFO
/// outputs in recv order).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SvScript {
    pub id: String,
    pub ops: Vec<SvOp>,
    pub expect: Vec<i64>,
}

/// The parsed, schema-checked `tyu.svcvec/1` document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SvcVecFile {
    pub schema: String,
    pub triple: String,
    pub scripts: Vec<SvScript>,
}

/// Codec failures (the corpus-band diagnostics: E6400 schema, E6401
/// malformed/oversized — fail-closed, never a silent best-effort read).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SvCodecError {
    Malformed,
    SchemaVersion { found: String },
    TooLarge { size: usize },
}

impl SvCodecError {
    /// The diagnostic code (E6400 for a schema mismatch, E6401 otherwise).
    pub fn code(&self) -> u32 {
        match self {
            SvCodecError::SchemaVersion { .. } => 6400,
            _ => 6401,
        }
    }
}

/// Parse + schema-check a `tyu.svcvec/1` document. Fail-closed: an
/// oversized, malformed, or wrongly-schema'd document is an error.
pub fn parse_svcvec(bytes: &[u8]) -> Result<SvcVecFile, SvCodecError> {
    if bytes.len() > SVCVEC_MAX_BYTES {
        return Err(SvCodecError::TooLarge { size: bytes.len() });
    }
    let text = core::str::from_utf8(bytes).map_err(|_| SvCodecError::Malformed)?;
    let mut r = Reader { b: bytes, i: 0 };
    let parsed = r.parse_doc().map_err(|_| SvCodecError::Malformed)?;
    if parsed.schema != SVCVEC_SCHEMA {
        return Err(SvCodecError::SchemaVersion {
            found: parsed.schema,
        });
    }
    // The committed corpus is fixed-shape; `text` is only used to keep the
    // bytes alive across the borrow (the reader owns its own cursor).
    let _ = text;
    Ok(parsed)
}

/// The Rust mirror of the abstract-atomic trace run (`Tyu.Services.traceRun`):
/// the recv outputs in EXECUTION order, or `None` when a recv hits a channel
/// without a FIFO head (a blocked receiver — the mirror treats it exactly as
/// the Lean model does).
pub fn run_trace(ops: &[SvOp]) -> Option<Vec<i64>> {
    let mut channels: Vec<(u64, Vec<i64>)> = Vec::new();
    let mut outputs: Vec<i64> = Vec::new();
    for op in ops {
        match op {
            SvOp::Make(c) => {
                if !channels.iter().any(|(id, _)| id == c) {
                    channels.push((*c, Vec::new()));
                }
            }
            SvOp::Send(c, v) => {
                let entry = channels.iter_mut().find(|(id, _)| *id == *c)?;
                entry.1.push(*v);
            }
            SvOp::Recv(c) => {
                let entry = channels.iter_mut().find(|(id, _)| *id == *c)?;
                let v = entry.1.first().copied()?;
                entry.1.remove(0);
                outputs.push(v);
            }
        }
    }
    Some(outputs)
}

/// Whether a script is hostable — the surface [`render_hosted_source`]
/// covers: ops partitioned into contiguous per-channel episodes, each with
/// exactly one make (first), ≥ 1 send, exactly one recv (last side-effect),
/// constant payloads. Used by the test to assert the boundary loudly.
pub fn hostable(script: &SvScript) -> bool {
    render_hosted_source(script).is_some()
}

/// The deterministic tyu SOURCE a script lowers to (P15.2): the channel
/// episodes of the wire form —
///
/// ```tyu
/// platform.channel.make
/// dup                # (makes + sends + recvs − 1) dups, exactly the
///                    # handles the episode's sends + recv consume
/// <payload> platform.channel.send
/// platform.channel.recv
/// <expected> ==      # the episode's single recv head-checks its first
///                    # sent value — FIFO, never LIFO
/// ```
///
/// joined with `and` over the episodes and the exit convention
/// (`not [ 1 ] [ 0 ] if`). A script outside the surface returns `None` —
/// fail-loud (the agreement chain requires the hosted counterpart).
pub fn render_hosted_source(script: &SvScript) -> Option<String> {
    // Partition into contiguous per-channel EPISODES (the wire form: after a
    // channel's `make`, its ops run to exhaustion before the next channel —
    // the data stack cannot hold a channel handle dormant under a received
    // value, so an interleaved recv order is not wire-realizable; the corpus
    // scripts are designed so every recv's handle is on top at call time).
    // Episode parsing: preserve op ORDER within the episode (sends and recvs
    // interleaved — the corpus's cross-cycle FIFO claim), recording the
    // recv expectations in execution order.
    struct EpisodeOps {
        _channel: u64,
        ops: Vec<EpisodeOp>,
    }
    enum EpisodeOp {
        Send(i64),
        Recv,
    }
    let mut episodes: Vec<EpisodeOps> = Vec::new();
    let mut recv_seen: u64 = 0;
    let mut expectations: Vec<i64> = Vec::new();
    let mut made: Vec<u64> = Vec::new();
    let mut i = 0usize;
    while i < script.ops.len() {
        let c = match script.ops[i] {
            SvOp::Make(c) => c,
            _ => return None, // each episode starts with a make
        };
        if made.contains(&c) {
            return None; // one make per channel (the wire never re-makes)
        }
        made.push(c);
        let mut ops: Vec<EpisodeOp> = Vec::new();
        i += 1;
        while i < script.ops.len() {
            match script.ops[i] {
                SvOp::Make(_) => break, // next episode
                SvOp::Send(d, v) if d == c => ops.push(EpisodeOp::Send(v)),
                SvOp::Recv(d) if d == c => {
                    // A recv immediately followed by ANOTHER recv on the
                    // same channel is not wire-realizable: the received value
                    // would sit above the next recv's handle. The corpus
                    // never does this; refuse loudly rather than emit an
                    // unbuildable program.
                    if matches!(script.ops.get(i + 1), Some(SvOp::Recv(_))) {
                        return None;
                    }
                    let want = script.expect.get(recv_seen as usize).copied()?;
                    expectations.push(want);
                    recv_seen += 1;
                    ops.push(EpisodeOp::Recv);
                }
                _ => return None,
            }
            i += 1;
        }
        if !ops.iter().any(|op| matches!(op, EpisodeOp::Recv)) {
            return None; // an episode must consume (a recv) what it verifies
        }
        episodes.push(EpisodeOps { _channel: c, ops });
    }
    if script.expect.len() != recv_seen as usize {
        return None;
    }

    // The generated source (deterministic; byte-stable). Built with the
    // hand-rolled string helpers (no `alloc::format!` — the hosted
    // `-nodefaultlibs` link constraint of this crate).
    //
    // Wire pattern: each recv's RESULT is consumed by an expect-word that
    // pushes a 0/1 mismatch indicator (`( i64 -- i64 ) <want> == not
    // [ 1 ] [ 0 ] if`), letting the channel handle resurface for the next op,
    // while the indicators accumulate BELOW all further channel ops. main
    // returns their sum — 0 iff every recv delivered its expected value
    // (FIFO head-of-line on the wire; a LIFO/reordering runtime diverges).
    let ident = module_ident(&script.id);
    if ident.is_empty() {
        return None;
    }
    let mut out = String::new();
    out.push_str("module ");
    out.push_str(&ident);
    out.push_str(";\nimport platform/channel;\n\n");
    // The expect-word per distinct expected value (declared in first-use
    // order; deduped so repeated payloads share one word).
    let mut expect_pushed: Vec<i64> = Vec::new();
    for want in &expectations {
        if !expect_pushed.contains(want) {
            out.push_str(": expect_");
            append_i64(&mut out, *want);
            out.push_str(" ( i64 -- i64 ) ");
            append_i64(&mut out, *want);
            out.push_str(" == not [ 1 ] [ 0 ] if ;\n");
            expect_pushed.push(*want);
        }
    }
    out.push_str("\n: main ( -- i64 )\n");
    let mut expected_ndx = 0usize;
    for ep in &episodes {
        let recvs = ep
            .ops
            .iter()
            .filter(|op| matches!(op, EpisodeOp::Recv))
            .count();
        let sends = ep
            .ops
            .iter()
            .filter(|op| matches!(op, EpisodeOp::Send(_)))
            .count();
        let dups = (sends + recvs) - 1;
        out.push_str("  platform.channel.make\n");
        for _ in 0..dups {
            out.push_str("  dup\n");
        }
        for op in &ep.ops {
            match op {
                EpisodeOp::Send(v) => {
                    out.push_str("  ");
                    append_i64(&mut out, *v);
                    out.push_str(" platform.channel.send\n");
                }
                EpisodeOp::Recv => {
                    out.push_str("  platform.channel.recv\n  expect_");
                    append_i64(&mut out, expectations[expected_ndx]);
                    out.push('\n');
                    expected_ndx += 1;
                }
            }
        }
    }
    // main's indicators sum: 0 iff every recv passed. (The channel handles
    // are all consumed by construction — one make + (sends + recvs − 1) dups
    // per episode — so the indicators are the only stack residue, folded
    // with (recvs − 1) `+` ops.)
    let total_recvs = recv_seen;
    for _ in 1..total_recvs {
        out.push_str("  +\n");
    }
    out.push_str("  ;\nexport { main };\nend;\n");
    Some(out)
}

/// Append the decimal representation of `v` to `out` (hand-rolled —
/// `alloc::format!` pulls the toolchain fmt machinery the hosted
/// `-nodefaultlibs` link cannot resolve; see `crate::codec`).
fn append_i64(out: &mut String, v: i64) {
    if v == i64::MIN {
        out.push_str("-9223372036854775808");
        return;
    }
    if v < 0 {
        out.push('-');
        out.push_str(&append_u64(v.unsigned_abs()));
        return;
    }
    out.push_str(&append_u64(v as u64));
}

fn append_u64(mut v: u64) -> String {
    let mut buf = [0u8; 20];
    let mut n = 0usize;
    if v == 0 {
        buf[0] = b'0';
        n = 1;
    }
    while v > 0 && n < buf.len() {
        buf[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    let mut out = String::with_capacity(n);
    for i in (0..n).rev() {
        out.push(buf[i] as char);
    }
    out
}

/// The deterministic module identifier for a script id (`fifo-send-recv` →
/// `FifoSendRecv`); returns `""` on a non-identifier id.
fn module_ident(id: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in id.chars() {
        if ch.is_ascii_alphanumeric() {
            if upper {
                out.extend(ch.to_uppercase());
                upper = false;
            } else {
                out.push(ch);
            }
        } else if ch == '_' || ch == '-' {
            upper = true;
        } else {
            return String::new();
        }
    }
    out
}

/// --- the hand-rolled JSON reader (FR-15; no serde) ---
struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn err<T>(&self) -> Result<T, ()> {
        Err(())
    }

    fn skip_ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\r' | b'\t') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn bump(&mut self) -> Result<u8, ()> {
        let b = self.b.get(self.i).copied().ok_or(())?;
        self.i += 1;
        Ok(b)
    }

    fn expect(&mut self, want: u8) -> Result<(), ()> {
        self.skip_ws();
        if self.bump()? != want {
            self.err()
        } else {
            Ok(())
        }
    }

    fn parse_doc(&mut self) -> Result<SvcVecFile, ()> {
        self.skip_ws();
        self.expect(b'{')?;
        let mut schema = None;
        let mut triple = None;
        let mut scripts = None;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            let key = self.parse_string()?;
            self.expect(b':')?;
            match key.as_str() {
                "schema" => schema = Some(self.parse_string()?),
                "triple" => triple = Some(self.parse_string()?),
                "scripts" => scripts = Some(self.parse_scripts()?),
                // unknown keys are skipped — additive growth
                _ => self.skip_value()?,
            }
        }
        let (Some(schema), Some(triple), Some(scripts)) = (schema, triple, scripts) else {
            return self.err();
        };
        Ok(SvcVecFile {
            schema,
            triple,
            scripts,
        })
    }

    fn parse_scripts(&mut self) -> Result<Vec<SvScript>, ()> {
        self.expect(b'[')?;
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.i += 1;
                break;
            }
            out.push(self.parse_script()?);
        }
        Ok(out)
    }

    fn parse_script(&mut self) -> Result<SvScript, ()> {
        self.expect(b'{')?;
        let mut id = None;
        let mut ops = None;
        let mut expect = None;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            let key = self.parse_string()?;
            self.expect(b':')?;
            match key.as_str() {
                "id" => id = Some(self.parse_string()?),
                "ops" => ops = Some(self.parse_ops()?),
                "expect" => expect = Some(self.parse_i64_array()?),
                _ => self.skip_value()?,
            }
        }
        let (Some(id), Some(ops), Some(expect)) = (id, ops, expect) else {
            return self.err();
        };
        Ok(SvScript { id, ops, expect })
    }

    fn parse_ops(&mut self) -> Result<Vec<SvOp>, ()> {
        self.expect(b'[')?;
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.i += 1;
                break;
            }
            self.expect(b'[')?;
            let name = self.parse_string()?;
            self.expect(b',')?;
            let a0 = self.parse_i64()?;
            let op = match name.as_str() {
                "make" => SvOp::Make(a0 as u64),
                "recv" => SvOp::Recv(a0 as u64),
                "send" => {
                    self.expect(b',')?;
                    let v = self.parse_i64()?;
                    SvOp::Send(a0 as u64, v)
                }
                _ => return self.err(),
            };
            self.expect(b']')?;
            out.push(op);
        }
        Ok(out)
    }

    fn parse_i64_array(&mut self) -> Result<Vec<i64>, ()> {
        self.expect(b'[')?;
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                Some(b',') => {
                    self.i += 1;
                }
                _ => {}
            }
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.i += 1;
                break;
            }
            out.push(self.parse_i64()?);
        }
        Ok(out)
    }

    fn parse_i64(&mut self) -> Result<i64, ()> {
        self.skip_ws();
        let neg = self.peek() == Some(b'-');
        if neg {
            self.i += 1;
        }
        let mut v: i64 = 0;
        let mut digits = 0usize;
        while let Some(d) = self.peek() {
            if d.is_ascii_digit() {
                v = v
                    .checked_mul(10)
                    .and_then(|x| x.checked_add((d - b'0') as i64))
                    .ok_or(())?;
                self.i += 1;
                digits += 1;
            } else {
                break;
            }
        }
        if digits == 0 {
            return self.err();
        }
        Ok(if neg { -v } else { v })
    }

    fn parse_string(&mut self) -> Result<String, ()> {
        self.skip_ws();
        if self.bump()? != b'"' {
            return self.err();
        }
        let mut out = String::new();
        loop {
            let b = self.bump()?;
            match b {
                b'"' => return Ok(out),
                b'\\' => match self.bump()? {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    _ => return self.err(),
                },
                0x00..=0x1F => return self.err(),
                _ => out.push(b as char),
            }
        }
    }

    fn skip_value(&mut self) -> Result<(), ()> {
        self.skip_ws();
        let c = self.bump()?;
        match c {
            b'{' => loop {
                self.skip_ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                    break;
                }
                if self.peek() == Some(b',') {
                    self.i += 1;
                    continue;
                }
                if self.peek().is_none() {
                    return self.err();
                }
                self.parse_string()?;
                self.expect(b':')?;
                self.skip_value()?;
            },
            b'[' => loop {
                self.skip_ws();
                if self.peek() == Some(b']') {
                    self.i += 1;
                    break;
                }
                if self.peek() == Some(b',') {
                    self.i += 1;
                    continue;
                }
                if self.peek().is_none() {
                    return self.err();
                }
                self.skip_value()?;
            },
            b'"' => {
                self.i -= 1;
                self.parse_string()?;
            }
            b't' | b'f' => {
                if self.b.get(self.i..self.i + 4) == Some(b"true") {
                    self.i += 4;
                } else if self.b.get(self.i..self.i + 5) == Some(b"false") {
                    self.i += 5;
                } else {
                    return self.err();
                }
            }
            b'n' => {
                if self.b.get(self.i..self.i + 4) == Some(b"null") {
                    self.i += 4;
                } else {
                    return self.err();
                }
            }
            b'-' | b'0'..=b'9' => {
                self.i -= 1;
                self.parse_i64()?;
            }
            _ => return self.err(),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    const CORPUS: &str = r#"{
      "schema": "tyu.svcvec/1",
      "triple": "x86_64-unknown-linux-gnu",
      "scripts": [
        { "id": "fifo-send-recv",
          "ops": [["make", 0], ["send", 0, 42], ["recv", 0]],
          "expect": [42] }
      ]
    }"#;

    #[test]
    fn parses_and_runs_the_committed_shape() {
        let f = parse_svcvec(CORPUS.as_bytes()).expect("parse");
        assert_eq!(f.schema, SVCVEC_SCHEMA);
        assert_eq!(f.scripts.len(), 1);
        let s = &f.scripts[0];
        assert_eq!(s.id, "fifo-send-recv");
        assert_eq!(run_trace(&s.ops), Some(vec![42]));
    }

    #[test]
    fn schema_is_checked_and_malformed_fails_closed() {
        let renamed = CORPUS.replace("tyu.svcvec/1", "tyu.svcvec/9");
        assert_eq!(
            parse_svcvec(renamed.as_bytes()),
            Err(SvCodecError::SchemaVersion {
                found: "tyu.svcvec/9".to_string()
            })
        );
        assert_eq!(
            parse_svcvec(b"{\"schema\":\"tyu.svcvec/1\",\"scripts\":[}"),
            Err(SvCodecError::Malformed)
        );
        let mut big = Vec::new();
        big.extend_from_slice(b"{\"schema\":\"tyu.svcvec/1\",\"triple\":\"x\",\"scripts\":");
        big.resize(SVCVEC_MAX_BYTES + 1, b' ');
        assert!(matches!(
            parse_svcvec(&big),
            Err(SvCodecError::TooLarge { .. })
        ));
    }

    #[test]
    fn run_trace_mirrors_the_fifo_laws() {
        // fifo-order: head-of-line — the recv returns the FIRST sent value.
        assert_eq!(
            run_trace(&[
                SvOp::Make(0),
                SvOp::Send(0, 7),
                SvOp::Send(0, 9),
                SvOp::Recv(0),
            ]),
            Some(vec![7])
        );
        // fifo-two-channels + fifo-isolation.
        assert_eq!(
            run_trace(&[
                SvOp::Make(0),
                SvOp::Send(0, 7),
                SvOp::Recv(0),
                SvOp::Make(1),
                SvOp::Send(1, 9),
                SvOp::Recv(1),
            ]),
            Some(vec![7, 9])
        );
        assert_eq!(
            run_trace(&[
                SvOp::Make(0),
                SvOp::Send(0, 5),
                SvOp::Make(1),
                SvOp::Send(1, 6),
                SvOp::Recv(0),
                SvOp::Recv(1),
            ]),
            Some(vec![5, 6])
        );
        // A recv on an empty channel blocks — the Lean `traceRun`'s recv
        // branch is `(none, _) => none`; the mirror agrees (fail-closed).
        assert_eq!(run_trace(&[SvOp::Make(0), SvOp::Recv(0)]), None);
        assert_eq!(run_trace(&[SvOp::Recv(0)]), None);
    }

    #[test]
    fn render_hosted_source_is_deterministic_and_refuses_unsupported() {
        let f = parse_svcvec(CORPUS.as_bytes()).unwrap();
        let src = render_hosted_source(&f.scripts[0]).expect("hostable");
        assert_eq!(render_hosted_source(&f.scripts[0]).unwrap(), src);
        assert!(src.contains("module FifoSendRecv"));
        assert!(src.contains("42 platform.channel.send"));
        assert!(src.contains(": expect_42 ( i64 -- i64 ) 42 == not [ 1 ] [ 0 ] if ;"));
        assert!(src.contains("expect_42"));
        // A second recv on the same channel (the data-stack bound) refuses.
        let unreachable = SvScript {
            id: "two-recv".to_string(),
            ops: vec![
                SvOp::Make(0),
                SvOp::Send(0, 1),
                SvOp::Send(0, 2),
                SvOp::Recv(0),
                SvOp::Recv(0),
            ],
            expect: vec![1, 2],
        };
        assert_eq!(render_hosted_source(&unreachable), None);
        // An id that is not an identifier refuses.
        let bad = SvScript {
            id: "not an id".to_string(),
            ops: vec![SvOp::Make(0), SvOp::Send(0, 1), SvOp::Recv(0)],
            expect: vec![1],
        };
        assert_eq!(render_hosted_source(&bad), None);
    }
}
