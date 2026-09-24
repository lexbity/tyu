//! Obligation id stability under unrelated edits (static-verification.md
//! NFR-4 / FR-9, slice P2).
//!
//! Obligation identity is `"<module>::<word>::<kind>::<occurrence>"` with
//! occurrence assigned in deterministic lowering order — never from source
//! spans. Editing one word (adding a stack op that changes its `high` fact)
//! plus comment/whitespace churn elsewhere MUST NOT renumber or rehash any
//! obligation of an untouched word, and MUST NOT change a touched word's
//! obligation ids or formulas (its computed facts may change).
//!
//! This is the facts-plumbing test too: the artifact's `facts.words[].high`
//! MUST move when the word's body changes, or the facts never reach the
//! artifact.

mod common;

use std::path::PathBuf;
use std::process::Command;

use verifier::codec::read_obl;

fn langc_exe() -> PathBuf {
    common::bin::resolve("langc")
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("tyu_obl_id_stability")
        .join(format!(
            "{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The ORIGINAL module: `f` exercises all three subtype-range sites
/// (param C1, cast C3, return C2); `g` is the untouched control word.
const ORIGINAL: &str = "\
module A;
subtype Percent = i64 range 0..100;

: f ( Percent -- Percent )
  1 + as Percent
;

: g ( i64 -- i64 )
  dup as Percent drop 1 +
;

end;
";

/// The MUTATED module: `f`'s body is rewritten to peak deeper
/// (`drop 1 1 1 1 + + +` vs `1 +` → word `high` 2 ⇒ 3) while the cast operand
/// stays an i64 and every subtype site is identical; `#` comments and blank
/// lines shift every span. `g` is byte-identical to the original.
const MUTATED: &str = "\
# churn: this comment shifts every line number below
module A;

# more churn
subtype Percent = i64 range 0..100;

: f ( Percent -- Percent )
  drop 1 1 1 1 + + + as Percent
;

: g ( i64 -- i64 )
  dup as Percent drop 1 +
;

end;
";

/// Compile with `--emit=obligations` and return the parsed artifact.
fn extract(tag: &str, source: &str) -> verifier::model::OblSet {
    let dir = fresh_dir(tag);
    let mod_path = dir.join("A.mod");
    std::fs::write(&mod_path, source).unwrap();
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out = Command::new(langc_exe())
        .arg("--emit=obligations")
        .arg(format!("--out-dir={}", out_dir.display()))
        .arg(mod_path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "langc --emit=obligations failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(out_dir.join("A.obl.json")).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    read_obl(&bytes).expect("artifact must round-trip through the codec")
}

/// Snapshot a word's obligations as (id, id_hash, formula) triples.
fn triples(set: &verifier::model::OblSet, word: &str) -> Vec<(String, String, String)> {
    set.obligations
        .iter()
        .filter(|o| o.site.word == word)
        .map(|o| {
            (
                o.id.clone(),
                o.id_hash.clone(),
                // Formula equality is what matters; format via Debug.
                format!("{:?}", o.formula),
            )
        })
        .collect()
}

#[test]
fn untouched_word_obligations_are_stable() {
    let original = extract("stable-orig", ORIGINAL);
    let mutated = extract("stable-mut", MUTATED);

    // g is byte-identical between the two sources: identical ids, hashes,
    // formulas.
    assert_eq!(triples(&original, "g"), triples(&mutated, "g"));
    // g's fact record is also untouched.
    let g_orig = original
        .facts
        .words
        .iter()
        .find(|w| w.name == "g")
        .expect("g fact present");
    let g_mut = mutated
        .facts
        .words
        .iter()
        .find(|w| w.name == "g")
        .expect("g fact present");
    assert_eq!(g_orig.high, g_mut.high);
}

#[test]
fn touched_word_ids_and_formulas_are_stable_but_high_fact_moves() {
    let original = extract("facts-orig", ORIGINAL);
    let mutated = extract("facts-mut", MUTATED);

    // f's obligation ids, hashes, and formulas are unaffected by the body
    // mutation + churn (its three sites — param, cast, return — are the same
    // sites, in the same order).
    assert_eq!(triples(&original, "f"), triples(&mutated, "f"));

    // The occurrence ordinals are the same deterministic IR order.
    let f_orig: Vec<u32> = original
        .obligations
        .iter()
        .filter(|o| o.site.word == "f")
        .map(|o| o.site.occurrence)
        .collect();
    let f_mut: Vec<u32> = mutated
        .obligations
        .iter()
        .filter(|o| o.site.word == "f")
        .map(|o| o.site.occurrence)
        .collect();
    assert_eq!(f_orig, f_mut);
    assert_eq!(f_orig, vec![0, 1, 2], "param, cast, return in IR order");

    // The facts MUST move: `dup` raises f's peak. This is the facts-plumbing
    // check — if high never reaches the artifact, the assertion fails.
    let high = |set: &verifier::model::OblSet| {
        set.facts
            .words
            .iter()
            .find(|w| w.name == "f")
            .expect("f fact present")
            .high
    };
    assert_ne!(
        high(&original),
        high(&mutated),
        "f's high fact must change when its body peaks differently"
    );

    // Sanity: the artifact actually carries both words' facts.
    assert_eq!(original.facts.words.len(), 2);
    assert_eq!(original.obligations.len(), 4); // f: 3 sites, g: 1 site
}

#[test]
fn ids_never_come_from_spans() {
    // The churn shifts every line in the source; the ids must not move.
    let original = extract("span-a", ORIGINAL);
    let mutated = extract("span-b", MUTATED);
    let ids = |set: &verifier::model::OblSet| {
        set.obligations
            .iter()
            .map(|o| o.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&original), ids(&mutated));
    // ...while the cast site's debug span DID move (line 7 -> the mutated
    // module's later line) — proving spans are debug-only, not identity.
    let line = |set: &verifier::model::OblSet| {
        set.obligations
            .iter()
            .find(|o| o.formula == verifier::model::Formula::InRange {
                value: verifier::model::Oel::Cast {
                    from: "i64".to_string(),
                    to: "Percent".to_string(),
                    arg: Box::new(verifier::model::Oel::Var {
                        name: "$top".to_string(),
                    }),
                },
                lo: 0,
                hi: 100,
            })
            .unwrap()
            .site
            .span
            .line
    };
    let l1 = line(&original);
    let l2 = line(&mutated);
    assert_ne!(l1, l2, "churn must shift the source line of the cast site");
}