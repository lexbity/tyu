//! Shared helpers for the wasm-repl integration tests (G53 harness +
//! session goldens): the §Q12 `SourceTable` for the hosted sysroot and the
//! fixture paths. One home — the per-file copies cannot drift.
//! (Not every test binary uses every helper.)

use std::path::PathBuf;

use wasm_repl::compile::SourceTable;

#[allow(dead_code)]
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

#[allow(dead_code)]
pub fn fixtures_dir() -> PathBuf {
    workspace_root().join("crates/execution-tests/fixtures")
}

#[allow(dead_code)]
pub fn sysroot_dir() -> PathBuf {
    workspace_root().join("sysroot")
}

/// The §Q12 source table for a hosted sysroot directory: an in-memory map
/// loaded from disk at harness time — the exact contract the browser's
/// sysroot blob (P5) will implement. Keys are canonical `.def` paths
/// (`platform/testio.def`).
pub struct MemTable {
    files: std::collections::BTreeMap<Vec<u8>, std::sync::Arc<[u8]>>,
}

impl SourceTable for MemTable {
    fn read(&self, path: &[u8]) -> Option<&[u8]> {
        self.files.get(path).map(|a| &**a)
    }
}

fn build_table(root: &std::path::Path) -> std::sync::Arc<MemTable> {
    let mut files = std::collections::BTreeMap::new();
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
                        files.insert(rel.into_bytes(), std::sync::Arc::from(bytes));
                    }
                }
            }
        }
    }
    std::sync::Arc::new(MemTable { files })
}

/// The §Q12 search directory set for the hosted sysroot, mirroring `langc`:
/// the generic root first, then the target-triple directory.
#[allow(dead_code)]
pub fn sysroot_search() -> Vec<std::sync::Arc<MemTable>> {
    vec![
        build_table(&sysroot_dir()),
        build_table(&sysroot_dir().join("x86_64-unknown-linux-gnu")),
    ]
}

/// Flatten the search tables into trait-object refs for `compile`.
#[allow(dead_code)]
pub fn search_refs(search: &[std::sync::Arc<MemTable>]) -> Vec<&dyn SourceTable> {
    search
        .iter()
        .map(|t| t.as_ref() as &dyn SourceTable)
        .collect()
}
