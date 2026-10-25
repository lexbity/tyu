//! W6 labs-corpus IR parity (PLAN-WASM-2 FR-32, AC-W6): for every RUNNABLE
//! example in the book's lab corpus, the session's `--emit=ir` render is
//! byte-identical to the native toolchain's `langc --emit=ir` for the same
//! buffer — the G53-b mechanism extended from the fixtures to the whole
//! teaching corpus. One renderer (DEC-13), two surfaces; any drift is a
//! defect.

use std::path::PathBuf;
use std::sync::Arc;
use std::process::Command;

use wasm_repl::compile::SourceTable;
use wasm_repl::examples_pack::{classify_examples, Expect, Kind, LabFile};

struct MemTable(std::collections::BTreeMap<Vec<u8>, std::sync::Arc<[u8]>>);

impl SourceTable for MemTable {
    fn read(&self, path: &[u8]) -> Option<&[u8]> {
        self.0.get(path).map(|a| &**a)
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn build_langc() {
    let s = Command::new(env!("CARGO"))
        .current_dir(workspace_root())
        .args(["build", "-q", "-p", "langc"])
        .status()
        .expect("cargo build");
    assert!(s.success(), "cargo build -p langc failed");
}

#[test]
fn labs_ir_render_is_byte_equal_to_langc() {
    build_langc();
    let langc = workspace_root().join("target").join("debug").join("langc");

    // The corpus, classified exactly as the packer does.
    let labs = workspace_root().join("docs/book/labs");
    let mut files: Vec<LabFile> = Vec::new();
    let mut stack = vec![labs.clone()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p
                    .extension()
                    .map(|x| x == "mod" || x == "toml")
                    .unwrap_or(false)
                {
                    let rel = p
                        .strip_prefix(&labs)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/");
                    let bytes = std::fs::read(&p).expect("lab reads");
                    files.push(LabFile {
                        path: rel,
                        content: String::from_utf8(bytes).expect("labs are UTF-8"),
                    });
                }
            }
        }
    }
    let packed = classify_examples(&files).expect("the corpus classifies totally");

    // The session's source tables (both roots, the CLI's search order).
    let sysroot = workspace_root().join("sysroot");
    let build_table = |root: &PathBuf| -> Arc<MemTable> {
        let mut t = std::collections::BTreeMap::new();
        let mut s = vec![root.clone()];
        while let Some(dir) = s.pop() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        s.push(p);
                    } else if p.extension().map(|x| x == "def").unwrap_or(false) {
                        let rel = p
                            .strip_prefix(root)
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .replace('\\', "/");
                        t.insert(
                            rel.into_bytes(),
                            std::sync::Arc::from(std::fs::read(&p).unwrap()),
                        );
                    }
                }
            }
        }
        Arc::new(MemTable(t))
    };
    let t0 = build_table(&sysroot);
    let t1 = build_table(&sysroot.join("x86_64-unknown-linux-gnu"));
    let refs: Vec<&dyn SourceTable> = vec![&*t0, &*t1];

    let mut checked = 0usize;
    let mut reds_checked = 0usize;
    for ex in &packed {
        let Kind::Runnable = ex.kind else {
            continue;
        };
        let lab_path = labs.join(format!("{}.mod", ex.id));

        // The native verdict: `langc --emit=ir` for the same buffer.
        let native = Command::new(&langc)
            .args([
                "--emit=ir",
                &format!("--sysroot={}", sysroot.display()),
            ])
            .arg(&lab_path)
            .output()
            .expect("langc --emit=ir runs");

        // The session's verdict + the pane's bytes (the same renderer).
        let mut session = wasm_repl::ReplSession::new(wasm_repl::SessionOpts::default());
        let result = session.transactional_submit(ex.source.as_bytes(), &refs);

        match &ex.expect {
            Expect::Ok => {
                // Compile-clean labs: the IR pane is byte-equal to the
                // native `--emit=ir` (DEC-13: one renderer, I-2).
                assert!(
                    native.status.success(),
                    "{}: langc --emit=ir failed:\n{}",
                    ex.id,
                    String::from_utf8_lossy(&native.stderr)
                );
                assert!(
                    matches!(result.status, wasm_repl::Status::Ok),
                    "{}: the session compiles the green lab ({:?})",
                    ex.id,
                    result.status
                );
                let rendered = session
                    .last_ir_text()
                    .expect("a successful compile renders IR");
                assert_eq!(
                    rendered, native.stdout,
                    "{}: the IR pane's bytes drifted from `langc --emit=ir`                      (DEC-13: one renderer)",
                    ex.id
                );
                checked += 1;
            }
            Expect::Diag { code } => {
                // Red labs: both surfaces reject, with the SAME code — the
                // IR parity pin extends to the failure mode (FR-11: native
                // codes verbatim).
                let wasm_repl::Status::CompileError { diag } = &result.status else {
                    panic!("{} packs red but the session says {:?}", ex.id, result.status);
                };
                assert!(
                    !native.status.success(),
                    "{} packs red but langc --emit=ir succeeded",
                    ex.id
                );
                if let Some(want) = code {
                    let stderr = String::from_utf8_lossy(&native.stderr);
                    assert!(
                        stderr.contains(&format!("E{want}")),
                        "{}: langc rejects with a different code than the packed                          expect ({want}):\n{stderr}",
                        ex.id
                    );
                    assert_eq!(
                        diag.code, *want,
                        "{}: session code E{} != packed expect {want}",
                        ex.id, diag.code
                    );
                }
                reds_checked += 1;
            }
            Expect::Trap { .. } => {
                panic!("{} packs a trap expectation — never emitted", ex.id)
            }
        }
    }
    assert!(
        checked >= 20,
        "the labs IR-parity leg covered the green corpus ({checked})"
    );
    assert!(
        reds_checked >= 20,
        "the labs IR-parity leg covered the red corpus ({reds_checked})"
    );
}
