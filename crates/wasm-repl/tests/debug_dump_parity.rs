//! G57(a) — the debug-surface parity pin
//!
//! Requires the `cli` feature (the repl-cli binary is required-features cli;
//! without it CARGO_BIN_EXE_repl-cli is absent and this test is inert). (PLAN-WASM-2 W4, I-11): the native
//! CLI's `debug-dump` JSON must equal, byte-for-byte, the snapshot the SAME
//! core API produces when a Rust test drives the identical session. One core
//! (`debug::dump_submission`), both surfaces; any divergence between them is
//! a defect.

#![cfg(feature = "cli")]

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use wasm_repl::compile::SourceTable;
use wasm_repl::{ReplSession, SessionOpts};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// The G57 fixture (one submission — see the session file's header).
const FIXTURE: &str = "test-goldens/repl/debug/console-fixture.session";

/// The submission line, read from the fixture (the last non-comment line).
fn fixture_submission() -> Vec<u8> {
    let script = std::fs::read_to_string(workspace_root().join(FIXTURE)).unwrap();
    script
        .lines()
        .rev()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .expect("fixture has a submission line")
        .as_bytes()
        .to_vec()
}

#[test]
fn cli_debug_dump_equals_the_core_api_snapshot() {
    let submission = fixture_submission();

    // The core-API side: a fresh session, the same sysroot search order the
    // CLI uses, one submission, then the SAME encoder the bin calls.
    let search = common::sysroot_search();
    let refs: Vec<&dyn SourceTable> = search
        .iter()
        .map(|t| t.as_ref() as &dyn SourceTable)
        .collect();
    let mut session = ReplSession::new(SessionOpts::default());
    session.transactional_submit(&submission, &refs);
    let core_line = wasm_repl::debug::dump_submission(&session, 1, &submission);

    // The CLI side: the bin's debug-dump mode (fresh session, same fixture).
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_repl-cli"));
    let out = Command::new(&exe)
        .arg("debug-dump")
        .arg(workspace_root().join(FIXTURE))
        .output()
        .expect("run repl-cli debug-dump");
    assert!(out.status.success(), "debug-dump exited non-zero");
    let text = String::from_utf8(out.stdout).unwrap();
    let mut lines = text.lines();
    let header = lines.next().expect("header");
    assert_eq!(
        header,
        format!("{{\"v\":1,\"engine\":{}}}", wasm_repl::ENGINE_VERSION)
    );
    let cli_line = lines.next().expect("one submission line");
    assert!(
        lines.next().is_none(),
        "the fixture has exactly one submission"
    );

    // I-11: byte-for-byte.
    assert_eq!(core_line, cli_line, "core API snapshot == CLI debug-dump");

    // And the snapshot carries the student-visible facts this fixture exists
    // to pin (frame order, stack depth, the trap's sig-vs-actual).
    for needle in [
        "\"status\":\"trapped 20\"",
        "\"stack_depth\":0",
        "\"word\":\"wrap\",\"depth\":1",
        "\"word\":\"add-safely\",\"depth\":2",
        "\"word\":\"no-sevens\",\"depth\":3",
        "\"kind\":\"sig-mismatch\",\"word\":\"add-safely\",\"declared\":\"( i64 -- i64 )\"",
        "\"value_hex\":\"0x0000000000000007\"",
    ] {
        assert!(core_line.contains(needle), "snapshot missing {needle}");
    }
}

// Keep the common module's helpers linked (the sysroot table above is the
// only use).
#[allow(dead_code)]
type KeepCommon = Arc<common::MemTable>;
