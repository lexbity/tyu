//! Source-line resolution and `Diagnostic` rendering.
//!
//! Provides [`SourceMap`] (a file-path → source-lines index) and
//! [`Diagnostic::render`] which produces a complete, copy-pasteable
//! trap report with word name, source context, claim text, and
//! data-stack depth information.
//!
//! Gated behind `feature = "std"`.

use std::collections::HashMap;
use std::format;
use std::path::{Path, PathBuf};
use std::string::{String, ToString};
use std::vec::Vec;

use crate::decode::Diagnostic;

// ---------------------------------------------------------------------------
// SourceMap
// ---------------------------------------------------------------------------

/// A lazy, indexed source-file cache.
///
/// Maps file system paths to indexed source text.  Lines are
/// 1-indexed: `get_line(path, 1)` returns the first line.
#[derive(Clone, Debug)]
pub struct SourceMap {
    files: HashMap<PathBuf, LineIndex>,
}

#[derive(Clone, Debug)]
struct LineIndex {
    source: String,
    line_starts: Vec<usize>,
}

impl LineIndex {
    fn new(source: String) -> Self {
        let mut line_starts = Vec::new();
        if !source.is_empty() {
            line_starts.push(0);
            for (idx, byte) in source.bytes().enumerate() {
                if byte == b'\n' && idx + 1 < source.len() {
                    line_starts.push(idx + 1);
                }
            }
        }
        Self {
            source,
            line_starts,
        }
    }

    fn get_line(&self, line_no: u32) -> Option<&str> {
        if line_no == 0 || line_no > self.line_starts.len() as u32 {
            return None;
        }
        let start = self.line_starts[(line_no - 1) as usize];
        let mut end = self
            .line_starts
            .get(line_no as usize)
            .map(|next| next.saturating_sub(1))
            .unwrap_or(self.source.len());
        let bytes = self.source.as_bytes();
        while end > start && matches!(bytes[end - 1], b'\n' | b'\r') {
            end -= 1;
        }
        self.source.get(start..end)
    }
}

impl SourceMap {
    /// Create an empty source map.
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    /// Read a source file from disk and index it.
    ///
    /// Returns `false` if the file cannot be read (missing, permission
    /// denied) — the renderer gracefully degrades rather than failing.
    pub fn add_file(&mut self, path: &Path) -> bool {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return false,
        };
        self.files
            .insert(path.to_path_buf(), LineIndex::new(content));
        true
    }

    /// Add source text directly (for in-memory sources used in tests).
    pub fn add_source(&mut self, path: &Path, source: &str) {
        self.files
            .insert(path.to_path_buf(), LineIndex::new(source.to_string()));
    }

    /// Retrieve a source line (1-indexed).
    ///
    /// Returns `None` if the file is not indexed or the line is out of range.
    pub fn get_line(&self, path: &Path, line_no: u32) -> Option<&str> {
        self.files.get(path)?.get_line(line_no)
    }
}

impl Default for SourceMap {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

impl Diagnostic {
    /// Render a complete, copy-pasteable trap report.
    ///
    /// When `source_path` and `source_map` are available, the report
    /// includes the offending source line with context.  Otherwise it
    /// shows the line number without source text.
    ///
    /// The output format is:
    ///
    /// ```text
    /// trap in 'main': ASSERT_FAIL (22) at line 5: ds=4/256
    ///  --> tests/fixtures/main.mod:5
    ///   |
    /// 5 |   assert
    ///   |
    /// ```
    pub fn render(&self, source_path: Option<&Path>, source_map: &SourceMap) -> String {
        let mut out = String::new();

        // Word name + claim text
        if let Some(ref name) = self.word_name {
            out.push_str(&format!("trap in '{}'", name));
        } else {
            out.push_str("trap (unknown word)");
        }
        out.push_str(&format!(": {} ({})", self.claim_text, self.trap_code));

        // Source line (if known)
        if self.source_line > 0 {
            out.push_str(&format!(" at line {}", self.source_line));
        }

        // Data-stack depth
        out.push_str(&format!(": ds={}", self.ds_depth));
        if self.ds_declared != crate::DS_DECLARED_UNKNOWN {
            out.push_str(&format!("/{}", self.ds_declared));
        }

        // Source context block (file + source line)
        if self.source_line > 0 {
            if let Some(path) = source_path {
                out.push_str(&format!("\n --> {}:{}", path.display(), self.source_line));
                if let Some(line) =
                    source_path.and_then(|p| source_map.get_line(p, self.source_line))
                {
                    out.push_str(&format!("\n  |\n{:>4} | {}\n  |", self.source_line, line));
                }
            }
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims;
    use crate::decode::Diagnostic;
    use crate::DS_DECLARED_UNKNOWN;

    fn source_map_with(content: &str) -> (SourceMap, PathBuf) {
        let path = PathBuf::from("test.mod");
        let mut sm = SourceMap::new();
        sm.add_source(&path, content);
        (sm, path)
    }

    #[test]
    fn source_map_indexes_single_buffer_by_offsets() {
        let (sm, path) = source_map_with("first\r\nsecond\nthird\n");
        assert_eq!(sm.get_line(&path, 1), Some("first"));
        assert_eq!(sm.get_line(&path, 2), Some("second"));
        assert_eq!(sm.get_line(&path, 3), Some("third"));
        assert_eq!(sm.get_line(&path, 0), None);
        assert_eq!(sm.get_line(&path, 4), None);
    }

    #[test]
    fn source_map_handles_empty_source() {
        let (sm, path) = source_map_with("");
        assert_eq!(sm.get_line(&path, 1), None);
    }

    fn make_diag(
        word: Option<&str>,
        trap_code: u16,
        line: u32,
        depth: u32,
        declared: u32,
    ) -> Diagnostic {
        Diagnostic {
            word_name: word.map(|s| s.to_string()),
            trap_code,
            claim_text: claims::claim_text(trap_code),
            valid: true,
            origin: crate::origin::IN_GUEST,
            source_line: line,
            ds_depth: depth,
            ds_declared: declared,
        }
    }

    // -------------------------------------------------------------------
    // Golden snapshots
    // -------------------------------------------------------------------

    #[test]
    fn render_named_word_with_source() {
        let (sm, path) = source_map_with("module Main;\nsubtype Small = i64 range 0..10;\n: main ( -- i64 )\n  100 as Small drop\n  0 ;\nend;\n");
        let diag = make_diag(Some("main"), 21, 4, 3, 256);
        let rendered = diag.render(Some(&path), &sm);

        assert!(rendered.contains("trap in 'main'"));
        assert!(rendered.contains("SUBTYPE_FAIL"));
        assert!(rendered.contains("test.mod:4"));
        assert!(rendered.contains("100 as Small drop"));
    }

    #[test]
    fn render_unknown_word_no_source() {
        let sm = SourceMap::new();
        let diag = make_diag(None, 10, 0, 16, DS_DECLARED_UNKNOWN);
        let rendered = diag.render(None, &sm);
        assert!(rendered.contains("unknown word"));
        assert!(rendered.contains("STACK_OVERFLOW"));
        assert!(rendered.contains("ds=16"));
        assert!(
            !rendered.contains("-->"),
            "no source path -> no arrow annotation"
        );
    }

    #[test]
    fn render_line_zero_no_source_context() {
        let (sm, path) = source_map_with(": main ( -- ) ;");
        let diag = make_diag(Some("main"), 5001, 0, 8, 128);
        let rendered = diag.render(Some(&path), &sm);
        assert!(rendered.contains("E_SUSPEND_FORBIDDEN"));
        assert!(rendered.contains("ds=8/128"));
        // No source context for line 0.
        assert!(!rendered.contains("-->"), "line 0 -> no source annotation");
        assert!(!rendered.contains("at line"));
    }

    #[test]
    fn render_declared_as_top_sentinel() {
        let (sm, path) = source_map_with(": f ( -- ) ;\n: g ( -- ) f ;\n");
        let diag = make_diag(Some("g"), 5100, 2, 12, DS_DECLARED_UNKNOWN);
        let rendered = diag.render(Some(&path), &sm);
        assert!(rendered.contains("E_STACK_UNBOUNDED"));
        assert!(rendered.contains("ds=12"));
        // No "/<N>" suffix when declared is unknown.
        assert!(!rendered.contains("ds=12/"));
        // Source context for line 2.
        assert!(rendered.contains("-->"));
        assert!(rendered.contains(": g ( -- ) f ;"));
    }

    #[test]
    fn render_uses_correct_trap_code_claim() {
        // Spot-check several trap codes.
        let cases = [
            (10, "STACK_OVERFLOW"),
            (22, "ASSERT_FAIL"),
            (5001, "E_SUSPEND_FORBIDDEN"),
            (5010, "E_ISO_DUP"),
            (5100, "E_STACK_UNBOUNDED"),
        ];
        for (code, expected_claim) in &cases {
            let diag = make_diag(Some("f"), *code, 0, 0, 0);
            let rendered = diag.render(None, &SourceMap::new());
            assert!(
                rendered.contains(expected_claim),
                "trap_code {} should yield claim '{expected_claim}', got: {rendered}",
                code,
            );
        }
    }
}
