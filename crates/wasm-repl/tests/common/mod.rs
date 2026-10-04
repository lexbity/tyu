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

// ---------------------------------------------------------------------------
// Minimal JSON reader (test-only): the fragvec corpus index is JSON, and the
// plan's dependency stance (§15 — "serde rejected; hand-rolled codecs") keeps
// even test-only serde out of this crate. Parses exactly the JSON grammar the
// corpus uses (objects, arrays, strings with escapes, integers, booleans);
// anything else is a parse error (fail-closed).
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub mod json {
    use std::collections::BTreeMap;

    #[derive(Clone, Debug, PartialEq)]
    pub enum Json {
        Bool(bool),
        Num(i64),
        Str(String),
        Arr(Vec<Json>),
        Obj(BTreeMap<String, Json>),
    }

    impl Json {
        pub fn as_str(&self) -> Option<&str> {
            match self {
                Json::Str(s) => Some(s),
                _ => None,
            }
        }
        pub fn as_i64(&self) -> Option<i64> {
            match self {
                Json::Num(n) => Some(*n),
                _ => None,
            }
        }
        pub fn as_bool(&self) -> Option<bool> {
            match self {
                Json::Bool(b) => Some(*b),
                _ => None,
            }
        }
        pub fn as_arr(&self) -> Option<&[Json]> {
            match self {
                Json::Arr(a) => Some(a),
                _ => None,
            }
        }
        pub fn get(&self, key: &str) -> Option<&Json> {
            match self {
                Json::Obj(m) => m.get(key),
                _ => None,
            }
        }
    }

    pub fn parse(src: &str) -> Result<Json, String> {
        let mut p = P {
            b: src.as_bytes(),
            i: 0,
        };
        p.ws();
        let v = p.value()?;
        p.ws();
        if p.i != p.b.len() {
            return Err(format!("trailing bytes at offset {}", p.i));
        }
        Ok(v)
    }

    struct P<'a> {
        b: &'a [u8],
        i: usize,
    }

    impl P<'_> {
        fn ws(&mut self) {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
        }
        fn peek(&self) -> Option<u8> {
            self.b.get(self.i).copied()
        }
        fn eat(&mut self, c: u8) -> Result<(), String> {
            if self.peek() == Some(c) {
                self.i += 1;
                Ok(())
            } else {
                Err(format!("expected '{}' at offset {}", c as char, self.i))
            }
        }
        fn lit(&mut self, s: &str) -> Result<(), String> {
            if self.b[self.i..].starts_with(s.as_bytes()) {
                self.i += s.len();
                Ok(())
            } else {
                Err(format!("expected '{s}' at offset {}", self.i))
            }
        }
        fn value(&mut self) -> Result<Json, String> {
            match self.peek().ok_or("unexpected end of input")? {
                b'{' => self.object(),
                b'[' => self.array(),
                b'"' => Ok(Json::Str(self.string()?)),
                b't' => {
                    self.lit("true")?;
                    Ok(Json::Bool(true))
                }
                b'f' => {
                    self.lit("false")?;
                    Ok(Json::Bool(false))
                }
                b'n' => {
                    self.lit("null")?;
                    Ok(Json::Num(0))
                }
                _ => self.number(),
            }
        }
        fn object(&mut self) -> Result<Json, String> {
            self.eat(b'{')?;
            let mut m = BTreeMap::new();
            self.ws();
            if self.peek() == Some(b'}') {
                self.i += 1;
                return Ok(Json::Obj(m));
            }
            loop {
                self.ws();
                let k = self.string()?;
                self.ws();
                self.eat(b':')?;
                self.ws();
                let v = self.value()?;
                m.insert(k, v);
                self.ws();
                match self.peek() {
                    Some(b',') => self.i += 1,
                    Some(b'}') => {
                        self.i += 1;
                        return Ok(Json::Obj(m));
                    }
                    _ => return Err(format!("expected ',' or '}}' at offset {}", self.i)),
                }
            }
        }
        fn array(&mut self) -> Result<Json, String> {
            self.eat(b'[')?;
            let mut a = Vec::new();
            self.ws();
            if self.peek() == Some(b']') {
                self.i += 1;
                return Ok(Json::Arr(a));
            }
            loop {
                self.ws();
                a.push(self.value()?);
                self.ws();
                match self.peek() {
                    Some(b',') => self.i += 1,
                    Some(b']') => {
                        self.i += 1;
                        return Ok(Json::Arr(a));
                    }
                    _ => return Err(format!("expected ',' or ']' at offset {}", self.i)),
                }
            }
        }
        fn string(&mut self) -> Result<String, String> {
            self.eat(b'"')?;
            let mut out = String::new();
            loop {
                match self.peek().ok_or("unterminated string")? {
                    b'"' => {
                        self.i += 1;
                        return Ok(out);
                    }
                    b'\\' => {
                        self.i += 1;
                        let e = self.peek().ok_or("unterminated escape")?;
                        self.i += 1;
                        match e {
                            b'"' => out.push('"'),
                            b'\\' => out.push('\\'),
                            b'/' => out.push('/'),
                            b'b' => out.push('\u{0008}'),
                            b'f' => out.push('\u{000C}'),
                            b'n' => out.push('\n'),
                            b'r' => out.push('\r'),
                            b't' => out.push('\t'),
                            b'u' => {
                                let hex = self
                                    .b
                                    .get(self.i..self.i + 4)
                                    .ok_or("truncated \\u escape")?;
                                let cp = u32::from_str_radix(
                                    std::str::from_utf8(hex).map_err(|e| e.to_string())?,
                                    16,
                                )
                                .map_err(|e| e.to_string())?;
                                self.i += 4;
                                out.push(char::from_u32(cp).ok_or("invalid \\u codepoint")?);
                            }
                            _ => return Err(format!("bad escape at offset {}", self.i)),
                        }
                    }
                    c => {
                        // UTF-8 continuation bytes pass through bytewise.
                        let start = self.i;
                        let len = if c < 0x80 {
                            1
                        } else if c >> 5 == 0b110 {
                            2
                        } else if c >> 4 == 0b1110 {
                            3
                        } else {
                            4
                        };
                        self.i += len;
                        let s = std::str::from_utf8(&self.b[start..self.i])
                            .map_err(|e| e.to_string())?;
                        out.push_str(s);
                    }
                }
            }
        }
        fn number(&mut self) -> Result<Json, String> {
            let start = self.i;
            if self.peek() == Some(b'-') {
                self.i += 1;
            }
            while self.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                self.i += 1;
            }
            let s = std::str::from_utf8(&self.b[start..self.i]).map_err(|e| e.to_string())?;
            s.parse::<i64>().map(Json::Num).map_err(|e| e.to_string())
        }
    }
}
