//! The REPL host driver (PLAN-WASM-1 P3): a thin test instrument that replays
//! session scripts the way the page (P5) will drive the session, emitting a
//! deterministic `tyu.repl-transcript/1` transcript. G56 replays
//! `test-goldens/repl/*.session` and requires the transcript to match the
//! committed `.expected` byte-for-byte.
//!
//! Session-script format (the test-instrument layer, never the language):
//!
//! ```text
//! # comments and blank lines are ignored
//! :fuel 1000            # set the per-phrase fuel (`0` = unlimited on host)
//! :elide / :noelide     # toggle L1 guard elision (§Q4)
//! :step N               # resume the fuel-suspended phrase with N ops (§Q8)
//! :stack                # print the persistent stack (bottom-first)
//! :ir                   # print the last-good `--emit=ir` hash (DEC-13)
//! :irdump               # debug: print the last-good `--emit=ir` text verbatim
//! :reset                # abandon the session
//! @repeat 4100 [1 ]     # expand to 4100 copies of `1 ` (caps goldens)
//! <anything else>       # a Tyu submission (definitions + optional phrase)
//! ```
//!
//! The `[1 2 +]`-style bracket in `@repeat` is NOT language syntax — it is
//! trimmed around the repeated text.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wasm_repl::compile::SourceTable;
use wasm_repl::{ReplSession, SessionOpts, Status, SubmitResult};

const TRANSCRIPT_SCHEMA: &str = "tyu.repl-transcript/1";

// ---------------------------------------------------------------------------
// The §Q12 source table (in-memory, from disk at startup — the P5 blob shape)
// ---------------------------------------------------------------------------

struct MemTable {
    files: BTreeMap<Vec<u8>, Arc<[u8]>>,
}

impl SourceTable for MemTable {
    fn read(&self, path: &[u8]) -> Option<&[u8]> {
        self.files.get(path).map(|a| &**a)
    }
}

fn build_table(root: &Path) -> Arc<MemTable> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().map(|x| x == "def").unwrap_or(false) {
                    let rel = p
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/");
                    if let Ok(bytes) = std::fs::read(&p) {
                        files.insert(rel.into_bytes(), Arc::from(bytes));
                    }
                }
            }
        }
    }
    Arc::new(MemTable { files })
}

fn sysroot_search() -> Vec<Arc<MemTable>> {
    let root = workspace_root().join("sysroot");
    vec![
        build_table(&root),
        build_table(&root.join("x86_64-unknown-linux-gnu")),
    ]
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

// ---------------------------------------------------------------------------
// Transcript writing (deterministic)
// ---------------------------------------------------------------------------

fn hex(b: &[u8]) -> String {
    let mut s = String::new();
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in data {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn stack_line(cells: &[wasm_repl::StackCell]) -> String {
    if cells.is_empty() {
        return "-".to_string();
    }
    cells
        .iter()
        .map(|c| {
            format!(
                "{}:{}",
                String::from_utf8_lossy(&c.ty_name),
                String::from_utf8_lossy(&c.value_hex)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn status_line(
    status: &Status,
    out: &[u8],
    trace: &Option<wasm_repl::Trace>,
    cells: &[wasm_repl::StackCell],
) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    // output first makes the retained-output-on-trap asymmetry visible.
    let _ = writeln!(s, "status: {}", status_word(status));
    if !out.is_empty() {
        let _ = writeln!(s, "output: {}", hex(out));
    }
    let _ = writeln!(s, "stack: {}", stack_line(cells));
    if let Some(t) = trace {
        let _ = writeln!(s, "trace: {}", t.normalize());
    }
    // The Q6/DEC-22 contract detail, as trailing fields ONLY when present
    // (existing goldens stay byte-stable except the contract-trap ones).
    if let Status::Trapped {
        detail: Some(d), ..
    } = status
    {
        let _ = write!(
            s,
            "detail: {} word={} declared={} actual=",
            String::from_utf8_lossy(d.kind.name()),
            String::from_utf8_lossy(&d.word),
            String::from_utf8_lossy(&d.declared),
        );
        if d.actual.is_empty() {
            let _ = writeln!(s, "-");
        } else {
            let rendered: Vec<String> = d
                .actual
                .iter()
                .map(|c| {
                    format!(
                        "{}:{}",
                        String::from_utf8_lossy(&c.ty_name),
                        String::from_utf8_lossy(&c.value_hex)
                    )
                })
                .collect();
            let _ = writeln!(s, "{}", rendered.join(" "));
        }
    }
    s
}

fn status_word(status: &Status) -> String {
    match status {
        Status::Ok => "ok".to_string(),
        Status::CompileError { diag } => format!("compile-error E{}", diag.code),
        Status::Trapped { code, .. } => format!("trapped {}", ir::trap_code_u32(*code)),
        Status::FuelExhausted { .. } => "fuel E9150".to_string(),
        Status::Internal { diag } => format!("internal E{}", diag.code),
        Status::Exited { code } => format!("exited {code}"),
        Status::DeviceError { code, aperture, .. } => match aperture {
            Some(ap) => format!("device-error E{code} ap{ap}"),
            None => format!("device-error E{code}"),
        },
    }
}

// ---------------------------------------------------------------------------
// Replay
// ---------------------------------------------------------------------------

/// Expand every `@repeat N [text]` directive anywhere in a script line to N
/// copies of `text` (a list-test-instrument shorthand for the caps goldens —
/// the session never sees `@repeat`). `@repeat 4100 [1 ]` → `1 1 … `.
fn expand_line(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(pos) = rest.find("@repeat") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + "@repeat".len()..];
        let n_end = after
            .find(|c: char| !c.is_ascii_digit() && c != ' ')
            .unwrap_or(after.len());
        let n: usize = after[..n_end].trim().parse().unwrap_or(0);
        let after = &after[n_end..];
        let Some(ob) = after.find('[') else {
            out.push_str("@repeat");
            rest = after;
            continue;
        };
        let Some(cb) = after[ob + 1..].find(']').map(|i| ob + 1 + i) else {
            out.push_str("@repeat");
            rest = after;
            continue;
        };
        let inner = after[ob + 1..cb].trim();
        for _ in 0..n {
            out.push_str(inner);
            out.push(' ');
        }
        rest = &after[cb + 1..];
    }
    out.push_str(rest);
    out
}

fn replay(session: &mut ReplSession, search: &[Arc<MemTable>], script: &str) -> String {
    use std::fmt::Write;

    let refs: Vec<&dyn SourceTable> = search
        .iter()
        .map(|t| t.as_ref() as &dyn SourceTable)
        .collect();
    let mut transcript = String::new();
    let _ = writeln!(transcript, "{TRANSCRIPT_SCHEMA}");
    let _ = writeln!(transcript, "spec x86_64-unknown-linux-gnu");

    let mut submit_count = 0usize;
    for raw in script.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let expanded = expand_line(line);
        let line_display = expanded.trim();
        if line_display.is_empty() {
            continue;
        }
        let _ = writeln!(transcript, "---");
        // Echo the SCRIPT line (compact), not the `@repeat` expansion.
        let _ = writeln!(transcript, "{}", line.trim());

        if let Some(rest) = line.strip_prefix(":fuel") {
            let n: u64 = rest.trim().parse().unwrap_or(0);
            session.set_fuel(n);
            let label = if n == 0 {
                "unlimited".to_string()
            } else {
                n.to_string()
            };
            let _ = writeln!(transcript, "fuel: {label}");
            continue;
        }
        if line == ":elide" {
            session.set_elide_guards(true);
            let _ = writeln!(transcript, "elide: on");
            continue;
        }
        if line == ":noelide" {
            session.set_elide_guards(false);
            let _ = writeln!(transcript, "elide: off");
            continue;
        }
        if let Some(rest) = line.strip_prefix(":step") {
            let n: u64 = rest.trim().parse().unwrap_or(0);
            let label = if n == 0 {
                "unlimited".to_string()
            } else {
                n.to_string()
            };
            let _ = writeln!(transcript, "step: {label}");
            let result: SubmitResult = session.step(n);
            let out: Vec<u8> = {
                let start = result.output_new.start as usize;
                let end = result.output_new.end as usize;
                session.out_total()[start..end].to_vec()
            };
            let _ = write!(
                transcript,
                "{}",
                status_line(&result.status, &out, &result.trace, &result.stack)
            );
            continue;
        }
        if line == ":stack" {
            let _ = writeln!(transcript, "stack: {}", stack_line(&session.stack_cells()));
            continue;
        }
        if line == ":ir" {
            let _ = writeln!(transcript, "ir: {}", ir_hash(session));
            continue;
        }
        if line == ":irdump" {
            // Debug-only: print the last-good `--emit=ir` text verbatim.
            if let Some(c) = session.compiled() {
                let mut rendered = Vec::new();
                let mut sink = wasm_repl::session::VecSink(&mut rendered);
                wasm_repl::render_ir(&mut sink, &c.module);
                let _ = writeln!(transcript, "{}", String::from_utf8_lossy(&rendered));
            }
            continue;
        }
        if line == ":reset" {
            session.reset();
            let _ = writeln!(transcript, "reset");
            continue;
        }

        // A submission.
        let result: SubmitResult = session.transactional_submit(line_display.as_bytes(), &refs);
        submit_count += 1;
        // The output bytes the session appended THIS submit (for the
        // transcript: report the new range's contents).
        let out: Vec<u8> = {
            let start = result.output_new.start as usize;
            let end = result.output_new.end as usize;
            session.out_total()[start..end].to_vec()
        };
        let _ = write!(
            transcript,
            "{}",
            status_line(&result.status, &out, &result.trace, &result.stack)
        );
        match &result.status {
            Status::CompileError { diag } | Status::Internal { diag } => {
                let _ = writeln!(
                    transcript,
                    "diag: E{} @{}..{} {}",
                    diag.code,
                    diag.span.start,
                    diag.span.end,
                    String::from_utf8_lossy(&diag.message)
                );
            }
            _ => {}
        }
    }
    let _ = writeln!(transcript, "---");
    let _ = writeln!(transcript, "submits: {submit_count}");
    transcript
}

fn ir_hash(session: &ReplSession) -> String {
    // The last-good `--emit=ir` render, hashed (§7.5 — the transcript carries
    // hashes, never the full text).
    let Some(c) = session.compiled() else {
        return "none".to_string();
    };
    let mut rendered = Vec::new();
    let mut sink = wasm_repl::session::VecSink(&mut rendered);
    wasm_repl::render_ir(&mut sink, &c.module);
    format!("{:016x}", fnv1a(&rendered))
}


// ---------------------------------------------------------------------------
// Debug-dump JSON (PLAN-WASM-2 W1, FR-27): per submission, the exact snapshot
// payloads the wasm ABI exports (`debug` projections) — the G57 fixture
// source. Hand-rolled deterministic JSON encoder; std allowed in the bin.
// ---------------------------------------------------------------------------

fn json_escape(b: &[u8]) -> String {
    let mut s = String::from("\"");
    for &c in b {
        match c {
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            b'\n' => s.push_str("\\n"),
            b'\r' => s.push_str("\\r"),
            b'\t' => s.push_str("\\t"),
            0x20..=0x7e => s.push(c as char),
            _ => s.push_str(&format!("\\u{:04x}", c)),
        }
    }
    s.push('"');
    s
}

fn json_cell(c: &wasm_repl::StackCell) -> String {
    format!(
        "{{\"ty_name\":{},\"value_hex\":{}}}",
        json_escape(&c.ty_name),
        json_escape(&c.value_hex)
    )
}

fn json_span(sp: frontend::span::Span) -> String {
    format!(
        "{{\"start\":{},\"len\":{}}}",
        sp.start,
        sp.end.saturating_sub(sp.start)
    )
}

fn json_detail(d: &wasm_repl::TrapDetail) -> String {
    let actual: Vec<String> = d.actual.iter().map(json_cell).collect();
    format!(
        "{{\"kind\":{},\"word\":{},\"declared\":{},\"actual\":[{}]}}",
        json_escape(d.kind.name()),
        json_escape(&d.word),
        json_escape(&d.declared),
        actual.join(",")
    )
}

fn json_event(e: &wasm_repl::debug::TraceEventView) -> String {
    use wasm_repl::debug::TraceEventView as Ev;
    match e {
        Ev::Output { start, len } => {
            format!("{{\"kind\":\"output\",\"start\":{start},\"len\":{len}}}")
        }
        Ev::Periph {
            ap,
            off,
            width,
            val,
            is_write,
        } => format!(
            "{{\"kind\":\"periph\",\"ap\":{ap},\"off\":{off},\"width\":{width},\"val\":{val:#x},\"is_write\":{is_write}}}"
        ),
        Ev::Call { word, depth } => format!(
            "{{\"kind\":\"call\",\"word\":{},\"depth\":{}}}",
            json_escape(word),
            depth
        ),
        Ev::Trap { code, span, detail } => {
            let d = detail
                .as_ref()
                .map(json_detail)
                .unwrap_or_else(|| "null".to_string());
            format!(
                "{{\"kind\":\"trap\",\"code\":{code},\"span\":{},\"detail\":{d}}}",
                json_span(*span)
            )
        }
        Ev::Device { code } => format!("{{\"kind\":\"device\",\"code\":{code}}}"),
        Ev::Internal { code } => format!("{{\"kind\":\"internal\",\"code\":{code}}}"),
        Ev::Fuel => "{\"kind\":\"fuel\"}".to_string(),
        Ev::Done { exit } => format!("{{\"kind\":\"done\",\"exit\":{exit}}}"),
    }
}

fn status_word_json(status: Option<&Status>) -> String {
    match status {
        None => "fresh".to_string(),
        Some(st) => status_word(st),
    }
}

/// The debug-dump mode: replay a session script, printing after every
/// submission the §5.2 snapshot JSON (frames / trace events / words /
/// regions / status) — the same `debug` projections the wasm ABI wraps
/// (R-E: one core, both surfaces, I-11).
fn debug_dump(session: &mut ReplSession, search: &[Arc<MemTable>], script: &str) -> String {
    use std::fmt::Write;
    let refs: Vec<&dyn SourceTable> = search
        .iter()
        .map(|t| t.as_ref() as &dyn SourceTable)
        .collect();
    let mut out = String::new();
    let _ = writeln!(out, "{{\"v\":1,\"engine\":{}}}", wasm_repl::ENGINE_VERSION);
    let mut submit_index = 0usize;
    for raw in script.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with(':')
            && [
                ":fuel", ":elide", ":noelide", ":step", ":stack", ":ir", ":irdump", ":reset",
            ]
            .iter()
            .any(|d| line.starts_with(d))
        {
            // A session directive (no submission to dump).
            continue;
        }
        let expanded = expand_line(line);
        let line_display = expanded.trim();
        if line_display.is_empty() {
            continue;
        }
        let _result = session.transactional_submit(line_display.as_bytes(), &refs);
        submit_index += 1;

        let compiled = session.compiled();
        let frames: Vec<String> = match session.suspended_state().zip(compiled) {
            Some((st, c)) => wasm_repl::debug::frames(c, st)
                .iter()
                .map(|f| {
                    format!(
                        "{{\"index\":{},\"word\":{},\"sig\":{},\"call_span\":{},\"locals\":[{}]}}",
                        f.index,
                        json_escape(&f.word),
                        json_escape(&f.sig),
                        json_span(f.call_span),
                        f.locals
                            .iter()
                            .map(json_cell)
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                })
                .collect(),
            None => Vec::new(),
        };
        let events: Vec<String> = match session.last_trace() {
            Some(t) => wasm_repl::debug::trace_events(t)
                .iter()
                .map(json_event)
                .collect(),
            None => Vec::new(),
        };
        let words: Vec<String> = match compiled {
            Some(c) => wasm_repl::debug::words(c, session.source())
                .iter()
                .map(|w| {
                    let origin = match &w.origin {
                        wasm_repl::debug::WordOrigin::Session { def_span } => format!(
                            "{{\"origin\":\"session\",\"def_span\":{}}}",
                            json_span(*def_span)
                        ),
                        wasm_repl::debug::WordOrigin::Sysroot { path } => format!(
                            "{{\"origin\":\"sysroot\",\"path\":{}}}",
                            json_escape(path)
                        ),
                    };
                    format!(
                        "{{\"name\":{},\"sig\":{},\"effects\":{},\"origin\":{}}}",
                        json_escape(&w.name),
                        json_escape(&w.sig),
                        json_escape(&w.effects),
                        origin
                    )
                })
                .collect(),
            None => Vec::new(),
        };
        let regions: Vec<String> = wasm_repl::debug::regions(session.heap())
            .iter()
            .filter(|r| r.alive)
            .map(|r| {
                format!(
                    "{{\"id\":{},\"off\":{:#x},\"len\":{:#x},\"cursor\":{:#x},\"kind\":{},\"alive\":{}}}",
                    r.id, r.off, r.len, r.cursor, r.kind, r.alive
                )
            })
            .collect();
        let st = wasm_repl::debug::status(session.stack_cells().len(), compiled, session.fuel());
        let _ = writeln!(
            out,
            "{{\"submit\":{},\"source\":{},\"status\":{},\"stack_depth\":{},\"words_count\":{},\"fuel\":{},\"frames\":[{}],\"trace_events\":[{}],\"words\":[{}],\"regions\":[{}]}}",
            submit_index,
            json_escape(line_display.as_bytes()),
            json_escape(status_word_json(session.last_status()).as_bytes()),
            st.stack_depth,
            st.word_count,
            st.fuel_ops,
            frames.join(","),
            events.join(","),
            words.join(","),
            regions.join(","),
        );
    }
    out
}

// ---------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, rest) = args
        .split_first()
        .map(|(m, r)| (m.as_str(), r))
        .unwrap_or(("help", &[]));

    match mode {
        "pack-sysroot" => {
            // Build `tyu-sysroot/1` from the hosted sysroot's `.def` sources
            // (§Q12): the SAME in-memory table the replay mode uses, encoded
            // content-hashed and path-sorted — deterministic, so G55's fresh
            // re-pack byte-compare is a valid drift check (R-7).
            let out = rest
                .first()
                .expect("usage: repl-cli pack-sysroot <out-blob>");
            let mut files: Vec<(Vec<u8>, Arc<[u8]>)> = Vec::new();
            let root = workspace_root().join("sysroot");
            let mut stack = vec![root.clone()];
            while let Some(dir) = stack.pop() {
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            stack.push(p);
                        } else if p.extension().map(|x| x == "def").unwrap_or(false) {
                            let rel = p
                                .strip_prefix(&root)
                                .unwrap()
                                .to_str()
                                .unwrap()
                                .replace('\\', "/");
                            if let Ok(bytes) = std::fs::read(&p) {
                                files.push((rel.into_bytes(), Arc::from(bytes)));
                            }
                        }
                    }
                }
            }
            files.sort_by(|a, b| a.0.cmp(&b.0));
            let refs: Vec<(&[u8], &[u8])> = files
                .iter()
                .map(|(p, c)| -> (&[u8], &[u8]) { (p.as_slice(), &**c) })
                .collect();
            let blob = wasm_repl::encode_blob(&refs).expect("pack");
            std::fs::write(out, &blob).expect("write blob");
            println!(
                "packed {} files, {} bytes → {}",
                files.len(),
                blob.len(),
                out
            );
        }
        "run" => {
            let file = rest.first().expect("usage: repl-cli run <session>");
            let script = std::fs::read_to_string(file).expect("read session");
            let mut session = ReplSession::new(SessionOpts::default());
            let search = sysroot_search();
            print!("{}", replay(&mut session, &search, &script));
        }
        "replay" => {
            let file = rest
                .first()
                .expect("usage: repl-cli replay <session> [--check]");
            let check = rest.iter().any(|a| a == "--check");
            let script = std::fs::read_to_string(file).expect("read session");
            let mut session = ReplSession::new(SessionOpts::default());
            let search = sysroot_search();
            let transcript = replay(&mut session, &search, &script);

            if check {
                let expected_path = expected_path_for(file);
                let expected = std::fs::read_to_string(&expected_path).unwrap_or_else(|_| {
                    panic!("missing expected transcript {}", expected_path.display())
                });
                if transcript != expected {
                    eprintln!(
                        "G56 mismatch: replay differs from {}",
                        expected_path.display()
                    );
                    eprintln!("--- expected ---\n{expected}");
                    eprintln!("--- actual ---\n{transcript}");
                    std::process::exit(1);
                }
                println!("G56 OK: {} matches {}", file, expected_path.display());
            } else {
                print!("{transcript}");
            }
        }
        "pack-examples" => {
            // Build `web/pkg/examples.js` from the book's lab corpus (W3,
            // DEC-18): every `.mod` classified, none dropped (an
            // unclassifiable file fails the build — the drift detector).
            let (labs_root, out) = match (rest.first(), rest.get(1)) {
                (Some(a), Some(b)) => (a.clone(), b.clone()),
                _ => {
                    eprintln!("usage: repl-cli pack-examples <labs-root> <out-file>");
                    std::process::exit(2);
                }
            };
            let mut files: Vec<wasm_repl::examples_pack::LabFile> = Vec::new();
            let mut stack = vec![PathBuf::from(&labs_root)];
            while let Some(dir) = stack.pop() {
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            stack.push(p);
                        } else if p.extension().map(|x| x == "mod" || x == "toml").unwrap_or(false) {
                            let rel = p
                                .strip_prefix(&labs_root)
                                .unwrap()
                                .to_str()
                                .unwrap()
                                .replace('\\', "/");
                            if let Ok(bytes) = std::fs::read(&p) {
                                if let Ok(text) = String::from_utf8(bytes) {
                                    files.push(wasm_repl::examples_pack::LabFile {
                                        path: rel,
                                        content: text,
                                    });
                                } else {
                                    eprintln!("pack-examples: non-UTF-8 lab: {rel}");
                                    std::process::exit(1);
                                }
                            }
                        }
                    }
                }
            }
            let out_bytes = wasm_repl::examples_pack::pack_examples(&files)
                .unwrap_or_else(|e| {
                    eprintln!("pack-examples: {e}");
                    std::process::exit(1);
                });
            std::fs::write(&out, &out_bytes).expect("write examples.js");
            let runnable = out_bytes.windows(b"kind: \"runnable\"".len())
                .filter(|w| w == b"kind: \"runnable\"").count();
            let native = out_bytes.windows(b"kind: \"native-only\"".len())
                .filter(|w| w == b"kind: \"native-only\"").count();
            println!("packed {} runnable / {native} native-only → {out} ({} bytes)", runnable, out_bytes.len());
        }
        "debug-dump" => {
            let file = rest.first().expect("usage: repl-cli debug-dump <session>");
            let script = std::fs::read_to_string(file).expect("read session");
            let mut session = ReplSession::new(SessionOpts::default());
            let search = sysroot_search();
            print!("{}", debug_dump(&mut session, &search, &script));
        }
        other => {
            eprintln!("usage: repl-cli (run|replay|debug-dump|pack-examples|pack-sysroot) ... ; got '{other}'");
            std::process::exit(2);
        }
    }
}

fn expected_path_for(session_file: &str) -> PathBuf {
    let p = Path::new(session_file);
    p.with_extension("expected")
}
