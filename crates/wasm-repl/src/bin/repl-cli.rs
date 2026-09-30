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
    s
}

fn status_word(status: &Status) -> String {
    match status {
        Status::Ok => "ok".to_string(),
        Status::CompileError { diag } => format!("compile-error E{}", diag.code),
        Status::Trapped { code, .. } => format!("trapped {}", ir::trap_code_u32(*code)),
        Status::FuelExhausted => "fuel".to_string(),
        Status::Exited { code } => format!("exited {code}"),
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
                let mut sink = Sink(&mut rendered);
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
        if let Status::CompileError { diag } = &result.status {
            let _ = writeln!(
                transcript,
                "diag: E{} @{}..{} {}",
                diag.code,
                diag.span.start,
                diag.span.end,
                String::from_utf8_lossy(&diag.message)
            );
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
    let mut sink = Sink(&mut rendered);
    wasm_repl::render_ir(&mut sink, &c.module);
    format!("{:016x}", fnv1a(&rendered))
}

struct Sink<'a>(&'a mut Vec<u8>);
impl frontend::parse::Output for Sink<'_> {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
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
        other => {
            eprintln!("usage: repl-cli (run|replay) <session> [--check]; got '{other}'");
            std::process::exit(2);
        }
    }
}

fn expected_path_for(session_file: &str) -> PathBuf {
    let p = Path::new(session_file);
    p.with_extension("expected")
}
