//! S2 Phase 18 — Superset equivalence & hardening.
//!
//! Proves that dynamic loading is a superset of static linking by running
//! a range of module variants through both paths and asserting identical
//! results.  Also fuzzes the loader with malformed containers.

mod common;

use lmod::validate::Container;
use common::*;

#[test]
fn phase18_equiv_constant_return() {
    let dir = fresh_dir("equiv_const");
    assert_eq!(dynamic_load_value("module Main;\n: main ( -- i64 ) 42 ;\nend;\n", &dir), 42);
    assert_eq!(dynamic_load_value("module Main;\n: main ( -- i64 ) 0 ;\nend;\n", &dir), 0);
    assert_eq!(dynamic_load_value("module Main;\n: main ( -- i64 ) 255 ;\nend;\n", &dir), 255);
}

#[test]
fn phase18_equiv_arithmetic() {
    let dir = fresh_dir("equiv_arith");
    let cases: [(&str, i32); 3] = [
        ("module Main;\n: main ( -- i64 ) 7 5 * ;\nend;\n", 35),
        ("module Main;\n: main ( -- i64 ) 100 23 - ;\nend;\n", 77),
        ("module Main;\n: main ( -- i64 ) 10 3 + 4 * ;\nend;\n", 52),
    ];
    for (source, expected) in &cases {
        let s = static_exit_code(source, &dir);
        assert_eq!(*expected, s, "static: expected {expected}, got {s}");
        let d = dynamic_load_value(source, &dir);
        assert_eq!(*expected as i64, d, "dynamic: expected {expected}, got {d}");
    }
}

#[test]
fn phase18_equiv_comparison() {
    let dir = fresh_dir("equiv_cmp");
    let cases: [(&str, i32); 4] = [
        ("module Main;\n: main ( -- i64 ) 5 3 > [ 1 ] [ 0 ] if ;\nend;\n", 1),
        ("module Main;\n: main ( -- i64 ) 3 5 > [ 1 ] [ 0 ] if ;\nend;\n", 0),
        ("module Main;\n: main ( -- i64 ) 3 3 == [ 1 ] [ 0 ] if ;\nend;\n", 1),
        ("module Main;\n: main ( -- i64 ) 3 5 < [ 1 ] [ 0 ] if ;\nend;\n", 1),
    ];
    for (source, expected) in &cases {
        let s = static_exit_code(source, &dir);
        assert_eq!(*expected, s);
        let d = dynamic_load_value(source, &dir);
        assert_eq!(*expected as i64, d);
    }
}

#[test]
fn phase18_equiv_control_flow() {
    let dir = fresh_dir("equiv_cf");
    let cases: [(&str, i32); 2] = [
        ("module Main;\n: main ( -- i64 ) 0 [ dup 5 < ] [ 1 + ] while ;\nend;\n", 5),
        ("module Main;\n: main ( -- i64 ) 5 [ dup 0 > ] [ 1 - ] while ;\nend;\n", 0),
    ];
    for (source, expected) in &cases {
        let s = static_exit_code(source, &dir);
        assert_eq!(*expected, s, "static: expected {expected}, got {s}");
        let d = dynamic_load_value(source, &dir);
        assert_eq!(*expected as i64, d, "dynamic: expected {expected}, got {d}");
    }
}

// -----------------------------------------------------------------------
// Loader hardening — fuzz with malformed containers
// -----------------------------------------------------------------------

#[test]
fn phase18_fuzz_container_random_mutations_no_panic() {
    let dir = fresh_dir("fuzz_containers");
    let lmod_path = compile_and_pack("module Main;\n: main ( -- i64 ) 42 ;\nend;\n", &dir);
    let base = std::fs::read(&lmod_path).unwrap();

    let mut rng = 98765u64;
    for _ in 0..512 {
        let mut bytes = base.clone();
        let n_mutations = (rng % 8) + 1;
        for _ in 0..n_mutations {
            let idx = (rng as usize) % bytes.len();
            bytes[idx] = bytes[idx].wrapping_add((rng & 0xff) as u8);
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        }
        let _result = Container::parse(&bytes); // must not panic
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
    }
}

#[test]
fn phase18_fuzz_container_truncation_no_panic() {
    let dir = fresh_dir("fuzz_trunc");
    let base = std::fs::read(&compile_and_pack(
        "module Main;\n: main ( -- i64 ) 42 ;\nend;\n", &dir)).unwrap();
    for len in 0..=base.len() {
        let _result = Container::parse(&base[..len]); // must not panic
    }
}

#[test]
fn phase18_fuzz_all_zeros_no_panic() {
    for size in [0, 1, 2, 4, 8, 16, 32, 64, 72, 128, 256, 1024, 4096] {
        let _result = Container::parse(&vec![0u8; size]); // must not panic
    }
}

#[test]
fn phase18_fuzz_all_ffs_no_panic() {
    for size in [0, 1, 72, 256, 1024] {
        let _result = Container::parse(&vec![0xffu8; size]); // must not panic
    }
}
