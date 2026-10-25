//! G58's behavioral expectation leg (PLAN-WASM-2 W5, §Q2/§5.4): the packer's
//! `expect` tags are derived from filenames and headers, never from running
//! the labs — so THIS test runs them. Every runnable example in the real
//! corpus is submitted to a fresh session: a green/interlude lab MUST
//! compile Ok; a red lab MUST be rejected, with the checker's code equal to
//! the packed `expect.code` whenever that is non-null. The labs corpus is
//! read-only input; a failure here is a classification finding, never a
//! golden-regen.

use std::path::PathBuf;
use std::sync::Arc;

use wasm_repl::compile::SourceTable;
use wasm_repl::examples_pack::{classify_examples, Expect, Kind, LabFile};

struct MemTable(std::collections::BTreeMap<Vec<u8>, Arc<[u8]>>);

impl SourceTable for MemTable {
    fn read(&self, path: &[u8]) -> Option<&[u8]> {
        self.0.get(path).map(|a| &**a)
    }
}

fn sysroot_search() -> Vec<Arc<MemTable>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("sysroot");
    let build = |root: &PathBuf| -> Arc<MemTable> {
        let mut files = std::collections::BTreeMap::new();
        let mut stack = vec![root.clone()];
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
        Arc::new(MemTable(files))
    };
    vec![build(&root), build(&root.join("x86_64-unknown-linux-gnu"))]
}

#[test]
fn every_runnable_example_pinned_behaviorally() {
    // The REAL corpus (read-only input).
    let labs = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("docs/book/labs");
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
    assert!(
        packed.len() >= 70,
        "the real corpus packs ({})",
        packed.len()
    );

    let search = sysroot_search();
    let refs: Vec<&dyn SourceTable> = search
        .iter()
        .map(|t| t.as_ref() as &dyn SourceTable)
        .collect();

    // The behavioral pin, over every runnable example. (Native-only
    // examples carry reasons and are not session-teachable by definition;
    // their rules were probe-verified in W3 and live in the table.)
    let mut greens = 0usize;
    let mut reds = 0usize;
    for ex in &packed {
        let Kind::Runnable = ex.kind else {
            assert!(!ex.reason.is_empty(), "native-only {} names its reason", ex.id);
            continue;
        };
        let mut session = wasm_repl::ReplSession::new(wasm_repl::SessionOpts::default());
        let status = session
            .transactional_submit(ex.source.as_bytes(), &refs)
            .status;
        match &ex.expect {
            Expect::Ok => {
                assert!(
                    matches!(status, wasm_repl::Status::Ok),
                    "{} packs green but the checker says {status:?}",
                    ex.id
                );
                greens += 1;
            }
            Expect::Diag { code } => {
                let wasm_repl::Status::CompileError { diag } = &status else {
                    panic!("{} packs red but the session says {status:?}", ex.id);
                };
                if let Some(want) = code {
                    assert_eq!(
                        diag.code, *want,
                        "{} packs expect code {want}, checker says E{}",
                        ex.id, diag.code
                    );
                }
                reds += 1;
            }
            Expect::Trap { .. } => {
                panic!("{} packs a trap expectation — the as-built classifier never emits those", ex.id)
            }
        }
    }
    assert!(greens >= 20, "green coverage ({greens})");
    assert!(reds >= 20, "red coverage ({reds})");
}
