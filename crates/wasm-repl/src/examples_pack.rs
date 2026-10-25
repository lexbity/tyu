//! The examples packer (PLAN-WASM-2 W3, DEC-18): the ONLY bridge between the
//! book's verified lab corpus (`docs/book/labs/**`) and the browser's
//! example surface. Turns the lab file set into the generated data module
//! `web/pkg/examples.js` — data only (I-7): source text + expectation
//! metadata, never executable content.
//!
//! Classification is a TOTAL, table-driven function of the corpus (§7.3):
//! every `.mod` is green-* / red-* / engine-* / interlude-*, or chapter-
//! native-only by rule, or explicitly excepted — an unclassifiable file is
//! a build error, never a silent drop. `expect` codes come from the lab's
//! OWN `# Expected: … E<pcode>` header — never guessed; a red lab without a
//! parsable code carries `code: null` and the G58 smoke then asserts only
//! non-Ok.
//!
//! The native-only rules (and the explicit exception table) record the
//! as-built ground truth of the browser REPL: module submissions COMPILE —
//! imports are signature-only, so a lab's `main` cannot be invoked and the
//! session cannot reproduce run-time traps. Each rule names its reason;
//! the emitted module carries them verbatim to the UI.
//!
//! Determinism (G58): sorted iteration, no map iteration order, a hand-rolled
//! JSON string encoder with a fixed escape set — two runs are byte-identical.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// One lab file as handed to [`pack_examples`] (the bin reads the
/// filesystem; the lib stays fs-free and testable).
#[derive(Clone)]
pub struct LabFile {
    /// Path relative to the labs root, `/`-separated:
    /// `ch01/green-01-hello.mod`, `ch10/green-01/App.mod`.
    pub path: String,
    pub content: String,
}

/// The example kind (§5.4): `runnable` compiles in the browser session
/// (green teaches "the checker accepts"; red teaches "the checker explains
/// itself"); `native-only` cannot be taught in the browser and says why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Runnable,
    NativeOnly,
}

/// The behavioral expectation (§5.4's closed set, as-built): `ok` — the
/// submission compiles clean; `diag` — the checker rejects it with `code`
/// (or some code, when `code` is `null`); `trap` is reserved for run-time
/// demos and is never emitted by the as-built classifier (see the module
/// docs: the session cannot run `main`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expect {
    Ok,
    Diag { code: Option<u32> },
    Trap { code: Option<u32> },
}

/// One classified example.
#[derive(Clone, Debug, PartialEq)]
pub struct Example {
    /// `<chapter-dir>/<stem>` (subdir layout: `<chapter>/<subdir>/<stem>`).
    pub id: String,
    /// The chapter number (from `chNN`).
    pub chapter: u32,
    pub title: String,
    pub kind: Kind,
    pub expect: Expect,
    /// Native-only reason (empty for runnable examples).
    pub reason: String,
    pub source: String,
}

/// The native-only rules and exceptions, as `(id-or-pattern, reason)`.
/// Chapter rules precede stem rules; the exception table is exact-id.
/// Ground truth: each rule was verified by submitting the lab to a session
/// (see the W3 slice report); the reasons are UI copy, carried verbatim.
fn native_only_reason(chapter: &str, id: &str, stem: &str) -> Option<String> {
    // Chapter rules.
    if chapter == "ch08" {
        return Some(
            "MMIO — needs a platform descriptor the browser REPL does not ship (E3640)"
                .to_string(),
        );
    }
    if chapter == "ch10" {
        return Some("multi-module lab (the session is one module)".to_string());
    }
    if chapter == "ch14" {
        return Some("book prose snippet (not a runnable REPL lab)".to_string());
    }
    // Engine labs: runtime-behavior demos. They compile cleanly, but the
    // point is the run-time trap — which the session cannot show (module
    // submissions compile; `main` is not invoked through import-only
    // interfaces).
    if stem.starts_with("engine-") {
        return Some(
            "runtime-trap demo — the session compiles modules but cannot run \
             main through import-only interfaces"
                .to_string(),
        );
    }
    // Exact exceptions (as-built divergences, each verified by probe).
    let reason = match id {
        // Interrupts are rejected at the REPL boundary (E9156).
        "ch11/green-01-compliant-share" => {
            "interrupts — rejected on the browser platform (E9156)"
        }
        // Tasks are rejected at the REPL boundary (E9156).
        "ch12/green-01-spawn-join-run" | "ch12/green-02-two-workers" => {
            "tasks — rejected on the browser platform (E9156)"
        }
        // Red only on the board target: the hosted interface ships
        // platform/channel, so the hosted-spec session compiles it clean.
        "ch12/red-03-channel-absent" => {
            "red only on the board target — the hosted interface ships \
             platform/channel"
        }
        // Book label drift: the current checker (native included) rejects
        // this lab with E3312 — surfaced to the book's owners as a finding.
        "ch09/green-01-factored-contract" => {
            "book label drift — the current checker rejects this lab (E3312)"
        }
        // Red at the native entry contract (main must return the exit
        // code, E1018) — the session's module submission compiles without
        // running main.
        "ch01/red-01-main-no-exit" => {
            "red at the native entry contract (E1018) — module submissions \
             compile without running main"
        }
        _ => return None,
    };
    Some(reason.to_string())
}

/// The chapter's manifest `ignore` list (§7.3: honored where present).
/// A minimal parser for the exact shape the corpus uses:
/// `[fixtures]` … `ignore = [ "a.mod", "b.mod", … ]`.
fn manifest_ignored(manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_ignore = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("ignore") {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                if rest.trim_start().starts_with('[') {
                    in_ignore = true;
                    continue;
                }
            }
        }
        if in_ignore {
            if let Some(item) = line.strip_prefix('"') {
                if let Some((name, _)) = item.split_once('"') {
                    out.push(name.to_string());
                    continue;
                }
            }
            if line.starts_with(']') {
                in_ignore = false;
            }
        }
    }
    out
}

/// The manifest's `[[fixture]]` name/file pairs (§7.3: names adopted as
/// titles when present) — `file` → `name`.
fn manifest_names(manifest: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with("name") {
            if let Some(rest) = line.split_once('=') {
                let v = rest.1.trim().trim_matches('"');
                if !rest.0.trim().is_empty() {
                    name = Some(v.to_string());
                }
            }
        } else if line.starts_with("file") {
            if let Some(rest) = line.split_once('=') {
                let v = rest.1.trim().trim_matches('"');
                if let Some(n) = name.take() {
                    out.push((v.to_string(), n));
                }
            }
        } else if line.starts_with("[[fixture]]") {
            name = None;
        }
    }
    out
}

/// The lab's own declared expectation, from its `# Expected:` header:
/// the first `E<pcode>` token (compile diagnostics) or `trap code <n>`
/// (run-time traps). `None` when the header declares neither — the
/// extractor MUST NOT guess (§5.4).
fn header_expect(content: &str) -> Option<Expect> {
    for line in content.lines() {
        if !line.starts_with('#') {
            break; // the header ends at the first non-comment line
        }
        if !line.contains("Expected") {
            continue;
        }
        // Run-time trap: "traps at run time with CONTRACT_FAIL (trap code 20".
        if let Some(pos) = line.find("trap code") {
            let digits: String = line[pos + "trap code".len()..]
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(n) = digits.parse::<u32>() {
                return Some(Expect::Trap { code: Some(n) });
            }
        }
        // Compile diagnostic: the first E+4-digits token.
        let bytes = line.as_bytes();
        let mut i = 0;
        while i + 5 <= bytes.len() {
            if bytes[i] == b'E'
                && bytes[i + 1..i + 5].iter().all(|b| b.is_ascii_digit())
            {
                let code = line[i + 1..i + 5].parse::<u32>().ok()?;
                return Some(Expect::Diag { code: Some(code) });
            }
            i += 1;
        }
    }
    None
}

/// The display title: `Lab <chapter>.<nn> — <slug>` for the numbered
/// convention, the manifest's adopted name when it names this file,
/// otherwise the bare stem (deterministic; never a guess).
fn title_for(stem: &str, chapter: u32, manifest_name: Option<&str>) -> String {
    if let Some(name) = manifest_name {
        return name.to_string();
    }
    // green-01-hello → "Lab 1.1 — hello"; red-03 → "Lab 3.3"; else the stem.
    let parts: Vec<&str> = stem.splitn(3, '-').collect();
    if parts.len() >= 2 {
        if let Ok(nn) = parts[1].parse::<u32>() {
            let slug = parts.get(2).copied().unwrap_or("");
            if slug.is_empty() {
                return format!("Lab {chapter}.{nn}");
            }
            return format!("Lab {chapter}.{nn} — {slug}");
        }
    }
    stem.to_string()
}

/// Classify and emit the whole corpus (§7.3). `files` is the complete
/// `.mod` file set under the labs root plus any `manifest.toml`s — the
/// caller reads them (sorted or not; the packer sorts). Errors name the
/// offending file: an unclassifiable lab is a build error (the drift
/// detector), never a silent drop.
pub fn pack_examples(files: &[LabFile]) -> Result<Vec<u8>, String> {
    // Manifests first: ignore lists and adopted names per chapter.
    let mut ignored: Vec<(String, Vec<String>)> = Vec::new(); // (chapter, names)
    let mut names: Vec<(String, Vec<(String, String)>)> = Vec::new(); // (chapter, file→name)
    for f in files {
        if f.path.ends_with("manifest.toml") {
            let chapter = chapter_of(&f.path)?;
            ignored.push((chapter.to_string(), manifest_ignored(&f.content)));
            names.push((chapter.to_string(), manifest_names(&f.content)));
        }
    }
    let is_ignored = |chapter: &str, file_name: &str| -> bool {
        ignored
            .iter()
            .filter(|(ch, _)| ch == chapter)
            .any(|(_, list)| list.iter().any(|n| n == file_name))
    };
    let manifest_name = |chapter: &str, file_name: &str| -> Option<String> {
        names
            .iter()
            .filter(|(ch, _)| ch == chapter)
            .flat_map(|(_, pairs)| pairs.iter())
            .find(|(f, _)| f == file_name)
            .map(|(_, n)| n.clone())
    };

    let mut mods: Vec<&LabFile> = files
        .iter()
        .filter(|f| f.path.ends_with(".mod"))
        .collect();
    mods.sort_by(|a, b| a.path.cmp(&b.path));

    let mut examples: Vec<Example> = Vec::new();
    let mut runnable = 0usize;
    let mut native_only = 0usize;
    for f in &mods {
        let chapter = chapter_of(&f.path)?;
        let file_name = f.path.rsplit('/').next().unwrap_or("");
        if is_ignored(&chapter, file_name) {
            continue;
        }
        let (stem, in_subdir) = stem_and_subdir(&f.path)?;
        let id = f.path.trim_end_matches(".mod").to_string();
        let title = title_for(&stem, chapter_num(&chapter), manifest_name(&chapter, file_name).as_deref());

        // Subdir layouts are chapter-native-only by rule (ch10 multi-module
        // pairs, ch14 prose snippets); any other chapter gaining subdirs is
        // a layout drift the build must flag.
        if in_subdir {
            // Subdir labs title as "<subdir>/<stem>" — the pair identity,
            // not the bare file stem.
            let title = format!(
                "{}/{}",
                &f.path[chapter.len() + 1..f.path.len() - stem.len() - ".mod".len() - 1],
                stem
            );
            let Some(reason) = native_only_reason(&chapter, &id, &stem) else {
                return Err(format!(
                    "unclassified subdir layout: {} (extend the chapter rules)",
                    f.path
                ));
            };
            examples.push(Example {
                id,
                chapter: chapter_num(&chapter),
                title,
                kind: Kind::NativeOnly,
                expect: Expect::Ok,
                reason,
                source: f.content.clone(),
            });
            native_only += 1;
            continue;
        }

        if let Some(reason) = native_only_reason(&chapter, &id, &stem) {
            examples.push(Example {
                id,
                chapter: chapter_num(&chapter),
                title,
                kind: Kind::NativeOnly,
                expect: Expect::Ok,
                reason,
                source: f.content.clone(),
            });
            native_only += 1;
            continue;
        }

        let expect = if stem.starts_with("red-") {
            match header_expect(&f.content) {
                Some(Expect::Diag { code }) => Expect::Diag { code },
                Some(Expect::Trap { code }) => Expect::Trap { code },
                _ => Expect::Diag { code: None },
            }
        } else if stem.starts_with("green-")
            || stem.starts_with("interlude-")
        {
            // Greens and interludes teach "compiles clean". A declared
            // header code (a red-style expectation on a green lab) is
            // layout drift — refused, never guessed around.
            match header_expect(&f.content) {
                None | Some(Expect::Ok) => Expect::Ok,
                Some(_) => {
                    return Err(format!(
                        "{}: green/interlude lab declares a red-style expectation",
                        f.path
                    ))
                }
            }
        } else {
            return Err(format!("unclassifiable lab: {} (stem convention)", f.path));
        };
        examples.push(Example {
            id,
            chapter: chapter_num(&chapter),
            title,
            kind: Kind::Runnable,
            expect,
            reason: String::new(),
            source: f.content.clone(),
        });
        runnable += 1;
    }

    examples.sort_by(|a, b| a.chapter.cmp(&b.chapter).then(a.id.cmp(&b.id)));
    emit(&examples)
        .map(|mut out| {
            // The coverage summary rides the front of the artifact as a
            // comment (machine-parseable by the G58 gate).
            let header = alloc::format!(
                "// GENERATED by `repl-cli pack-examples` — do not edit. Hash pinned in artifacts.js.\n\
                 // classified: {runnable} runnable / {native_only} native-only / 0 unclassified\n"
            );
            out.splice(0..0, header.as_bytes().iter().copied());
            out
        })
}

fn chapter_num(chapter: &str) -> u32 {
    chapter[2..].parse().unwrap_or(0)
}

fn chapter_of(path: &str) -> Result<String, String> {
    let first = path.split('/').next().unwrap_or("");
    if first.len() == 4 && first.starts_with("ch") && first[2..].bytes().all(|b| b.is_ascii_digit())
    {
        Ok(first.to_string())
    } else {
        Err(format!("file outside a chNN directory: {path}"))
    }
}

/// `(stem, in_subdir)` for a `.mod` path (`ch01/x.mod` → ("x", false)).
fn stem_and_subdir(path: &str) -> Result<(String, bool), String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 {
        return Err(format!("malformed lab path: {path}"));
    }
    let base = parts[parts.len() - 1];
    Ok((base.trim_end_matches(".mod").to_string(), parts.len() > 2))
}

// ---------------------------------------------------------------------------
// The deterministic JSON emitter (§5.4 / §7.3): a fixed escape set, sorted
// input, no map iteration — two runs are byte-identical (G58).
// ---------------------------------------------------------------------------

fn emit(examples: &[Example]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    out.extend_from_slice(b"export const EXAMPLES = [\n");
    for e in examples {
        out.extend_from_slice(b"  { ");
        write_kv(&mut out, "id", &e.id);
        out.extend_from_slice(b", ");
        write_num(&mut out, "chapter", e.chapter);
        out.extend_from_slice(b", ");
        write_kv(&mut out, "title", &e.title);
        out.extend_from_slice(b", kind: \"");
        out.extend_from_slice(match e.kind {
            Kind::Runnable => b"runnable".as_slice(),
            Kind::NativeOnly => b"native-only".as_slice(),
        });
        out.extend_from_slice(b"\", ");
        match e.kind {
            Kind::NativeOnly => {
                write_kv(&mut out, "reason", &e.reason);
                out.extend_from_slice(b", ");
            }
            Kind::Runnable => {}
        }
        match &e.expect {
            Expect::Ok => out.extend_from_slice(b"expect: { outcome: \"ok\" }"),
            Expect::Diag { code } => {
                out.extend_from_slice(b"expect: { outcome: \"diag\", code: ");
                write_code(&mut out, *code);
                out.extend_from_slice(b" }");
            }
            Expect::Trap { code } => {
                out.extend_from_slice(b"expect: { outcome: \"trap\", code: ");
                write_code(&mut out, *code);
                out.extend_from_slice(b" }");
            }
        }
        out.extend_from_slice(b",\n    source: ");
        write_kv(&mut out, "", &e.source);
        out.extend_from_slice(b" },\n");
    }
    out.extend_from_slice(b"];\n");
    Ok(out)
}

fn write_code(out: &mut Vec<u8>, code: Option<u32>) {
    match code {
        Some(n) => out.extend_from_slice(n.to_string().as_bytes()),
        None => out.extend_from_slice(b"null"),
    }
}

fn write_num(out: &mut Vec<u8>, key: &str, n: u32) {
    out.extend_from_slice(key.as_bytes());
    out.extend_from_slice(b": ");
    out.extend_from_slice(n.to_string().as_bytes());
}

fn write_kv(out: &mut Vec<u8>, key: &str, value: &str) {
    if !key.is_empty() {
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(b": ");
    }
    out.push(b'"');
    for c in value.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(alloc::format!("\\u{:04x}", c as u32).as_bytes())
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn lab(path: &str, content: &str) -> LabFile {
        LabFile {
            path: path.to_string(),
            content: content.to_string(),
        }
    }

    const GREEN: &str = "# green\n# Expected: silent clean run.\nmodule G;\nend;\n";
    const RED: &str = "# red\n# Expected: rejected at compile time — E3202.\nmodule R;\nend;\n";
    const RED_NO_CODE: &str = "# red\n# Expected: rejected at compile time.\nmodule R2;\nend;\n";
    const ENGINE: &str = "# engine\n# Expected: compiles cleanly; traps at run time (trap code 20).\nmodule E;\nend;\n";

    #[test]
    fn classifies_by_convention_with_header_codes() {
        let files = vec![
            lab("ch02/green-01-x.mod", GREEN),
            lab("ch02/red-01-underflow.mod", RED),
            lab("ch03/red-02-mystery.mod", RED_NO_CODE),
            lab("ch01/engine-broken.mod", ENGINE),
        ];
        let out = pack_examples(&files).expect("pack");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("id: \"ch02/green-01-x\", chapter: 2"));
        assert!(text.contains("expect: { outcome: \"ok\" }"));
        assert!(text.contains("expect: { outcome: \"diag\", code: 3202 }"));
        // A red without a parsable header code is `null` — never a guess.
        assert!(text.contains("code: null"));
        // Engine labs are native-only with the runtime-demo reason.
        assert!(text.contains("runtime-trap demo"));
        assert!(text.contains("classified: 3 runnable / 1 native-only / 0 unclassified"));
    }

    #[test]
    fn emit_is_byte_deterministic() {
        let files = vec![
            lab("ch02/red-01-underflow.mod", RED),
            lab("ch01/green-01-x.mod", GREEN),
            lab("ch01/engine-broken.mod", ENGINE),
        ];
        let a = pack_examples(&files).unwrap();
        let b = pack_examples(&files).unwrap();
        assert_eq!(a, b, "two runs are byte-identical (G58)");
        // Input order does not matter: sorted before emit.
        let reordered = vec![
            files[1].clone(),
            files[2].clone(),
            files[0].clone(),
        ];
        let c = pack_examples(&reordered).unwrap();
        assert_eq!(a, c, "sorted emit is input-order independent");
    }

    #[test]
    fn manifest_ignore_and_names_are_honored() {
        let manifest = "# ch05 fixtures\n[fixtures]\nignore = [\n  \"red-01-nope.mod\",\n]\n\n[[fixture]]\nname = \"scratch_rt\"\nfile = \"green-01-scratch.mod\"\n";
        let files = vec![
            lab("ch05/manifest.toml", manifest),
            lab("ch05/green-01-scratch.mod", GREEN),
            lab("ch05/red-01-nope.mod", RED),
            lab("ch05/red-02-kept.mod", RED),
        ];
        let out = pack_examples(&files).expect("pack");
        let text = String::from_utf8(out).unwrap();
        // The ignored red is skipped; the manifest name is the title.
        assert!(!text.contains("red-01-nope"));
        assert!(text.contains("title: \"scratch_rt\""));
        assert!(text.contains("id: \"ch05/red-02-kept\""));
    }

    #[test]
    fn subdir_layouts_are_native_only_by_rule() {
        let files = vec![
            lab("ch10/green-01/App.mod", GREEN),
            lab("ch10/green-01/Math.mod", GREEN),
            lab("ch14/anon/Anon.mod", GREEN),
        ];
        let out = pack_examples(&files).expect("pack");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("multi-module lab"));
        assert!(text.contains("book prose snippet"));
    }

    #[test]
    fn exceptions_are_exact_id() {
        let files = vec![
            lab("ch12/red-03-channel-absent.mod", RED),
            lab("ch12/red-02-other.mod", RED),
        ];
        let out = pack_examples(&files).expect("pack");
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("board target"));
        assert!(text.contains("id: \"ch12/red-02-other\", chapter: 12"));
    }

    #[test]
    fn unclassifiable_is_a_build_error() {
        let files = vec![lab("ch06/interlude-deep-stack.mod", GREEN), lab("ch06/mystery-lab.mod", GREEN)];
        let err = pack_examples(&files).unwrap_err();
        assert!(err.contains("unclassifiable"), "{err}");
    }

    #[test]
    fn files_outside_ch_dirs_are_refused() {
        let files = vec![lab("labs/stray.mod", GREEN)];
        assert!(pack_examples(&files).is_err());
    }
}
