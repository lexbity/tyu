//! Property test for M4 borrow exclusivity (S-9).
//!
//! Enumerates operation sequences up to length N against an independent
//! oracle that encodes the D-4/D-5/D-9 rules.
//!
//! The oracle is ~50 lines and covers:
//!   - &!a vs &!a → E5021
//!   - &!a vs &a  → E5021
//!   - &a vs &a   → OK (shared+shared)
//!   - sequential re-borrow → OK
//!   - dup of &!a → E5022
//!   - bind &!a, ref → OK; second ref → E5023
//!   - bind &a, ref, ref → OK (shared non-linear)

use std::process::Command;
use std::thread;

fn langc_exe() -> std::path::PathBuf {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap();
    workspace.join("target").join("debug").join("langc")
}

fn repo_sysroot() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().join("sysroot")
}

const OPS: &[&str] = &[
    "MintMut",     // &!a
    "MintShared",  // &a
    "Consume",     // drop
    "Dup",         // dup
    "Bind",        // => x
    "Ref",         // x
];

/// Oracle: given a sequence of operation names, returns:
///   None → should compile
///   Some(u32) → should produce this error code
fn oracle(ops: &[&str]) -> Option<u32> {
    let mut live_mut = false;
    let mut live_shr = false;
    let mut stack_depth: usize = 0;
    let mut local_is_mut: Option<bool> = None;
    let mut local_used = false; // whether the local was already consumed

    for &op in ops {
        match op {
            "MintMut" => {
                if live_mut || live_shr { return Some(5021); }
                live_mut = true;
                stack_depth += 1;
            }
            "MintShared" => {
                if live_mut { return Some(5021); }
                live_shr = true;
                stack_depth += 1;
            }
            "Consume" => {
                if stack_depth == 0 { return Some(3202); }
                stack_depth -= 1;
                if stack_depth == 0 {
                    live_mut = false;
                    live_shr = false;
                }
            }
            "Dup" => {
                if stack_depth == 0 { return Some(3202); }
                if live_mut { return Some(5022); } // dup of &! is error; & dup is OK
                stack_depth += 1;
            }
            "Bind" => {
                if stack_depth == 0 { return Some(3202); }
                local_is_mut = Some(live_mut);
                local_used = false;
                // The borrow is now shelved in the local, but still live
                // (S-5: local_place participates in scan_conflict).
                // live_mut/live_shr remain true: the root is still occupied.
                stack_depth -= 1;
            }
            "Ref" => {
                match local_is_mut {
                    None => return Some(3210), // WordNotFound (x was never bound)
                    Some(true) => {
                        if local_used { return Some(5023); } // second ref
                        local_used = true;
                        local_is_mut = None; // consumed
                        live_mut = true;
                        stack_depth += 1;
                    }
                    Some(false) => {
                        live_shr = true;
                        stack_depth += 1;
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Build and compile a tyu source for operation sequence, return compiler error code.
fn compile_seq(ops: &[&str], dir: &std::path::Path) -> Option<u32> {
    let mut body = Vec::new();
    let mut net_stack = 0i32;
    for &op in ops {
        match op {
            "MintMut" | "MintShared" | "Dup" | "Ref" => { body.extend_from_slice(match op {
                "MintMut" => b"&!a ",
                "MintShared" => b"&a ",
                "Dup" => b"dup ",
                "Ref" => b"x ",
                _ => unreachable!(),
            }); net_stack += 1; }
            "Consume" => { body.extend_from_slice(b"drop "); net_stack -= 1; }
            "Bind" => { body.extend_from_slice(b"=> x "); net_stack -= 1; }
            _ => {}
        }
    }
    // Drain any remaining stack items to keep the lock body stack-neutral.
    for _ in 0..net_stack {
        body.extend_from_slice(b"drop ");
    }
    body.extend_from_slice(b"] 0 ;\nend;\n");

    let mut src = Vec::new();
    src.extend_from_slice(b"module Main;\n\
import platform/linux { };\n\
resource a : u32 = 0;\n\
: main ( -- i64 )\n\
  a lock [ ");
    src.extend_from_slice(&body);

    let mod_path = dir.join("seq.mod");
    std::fs::write(&mod_path, &src).unwrap();
    let out = Command::new(langc_exe())
        .current_dir(dir)
        .arg("--emit=ir")
        .arg(format!("--sysroot={}", repo_sysroot().to_string_lossy()))
        .arg(mod_path.to_str().unwrap())
        .output()
        .unwrap();

    if out.status.success() {
        None
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        stderr.split("error[E")
            .nth(1)
            .and_then(|s| s.split(']').next())
            .and_then(|s| s.parse::<u32>().ok())
    }
}

fn test_all_sequences(max_len: usize, dir: &std::path::Path) {
    // Enumerate all sequences of length 1..=max_len.
    // For max_len=4: 6+36+216+1296 = 1554 sequences.
    let mut tested = 0u64;
    let mut mismatches = 0u64;

    // Generate all sequences iteratively.
    let mut stack: Vec<Vec<&str>> = vec![vec![]];
    while let Some(seq) = stack.pop() {
        let len = seq.len();
        if len > 0 {
            let oracle_verdict = oracle(&seq);
            let compiler_code = compile_seq(&seq, dir);
            match (oracle_verdict, compiler_code) {
                (None, None) => {}
                (Some(o), Some(c)) if o == c => {}
                (Some(o), Some(c)) => {
                    eprintln!("MISMATCH [{}]: oracle E{o}, compiler E{c}", seq.join(" "));
                    mismatches += 1;
                }
                (Some(o), None) => {
                    eprintln!("MISMATCH [{}]: oracle E{o}, compiler accepted", seq.join(" "));
                    mismatches += 1;
                }
                (None, Some(c)) => {
                    eprintln!("MISMATCH [{}]: oracle OK, compiler E{c}", seq.join(" "));
                    mismatches += 1;
                }
            }
            tested += 1;
        }
        if len < max_len {
            for &op in OPS {
                let mut next = seq.clone();
                next.push(op);
                stack.push(next);
            }
        }
    }

    eprintln!("borrow_property: {tested} sequences tested (len ≤ {max_len}), {mismatches} mismatches");
    assert_eq!(mismatches, 0, "{mismatches} oracle/compiler mismatches found");
}

fn spawn_test(f: impl FnOnce() + Send + 'static) {
    thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn property_length_1() {
    let dir = std::env::temp_dir().join("tyu_borrow_prop_1");
    let _ = std::fs::create_dir_all(&dir);
    let d = dir.clone();
    spawn_test(move || test_all_sequences(1, &d));
}

#[test]
fn property_length_2() {
    let dir = std::env::temp_dir().join("tyu_borrow_prop_2");
    let _ = std::fs::create_dir_all(&dir);
    let d = dir.clone();
    spawn_test(move || test_all_sequences(2, &d));
}

#[test]
fn property_length_3() {
    let dir = std::env::temp_dir().join("tyu_borrow_prop_3");
    let _ = std::fs::create_dir_all(&dir);
    let d = dir.clone();
    spawn_test(move || test_all_sequences(3, &d));
}
