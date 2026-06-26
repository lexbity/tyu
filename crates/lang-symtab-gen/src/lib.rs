use std::collections::BTreeMap;
use std::fmt::Write as _;

use object::{Object, ObjectSymbol};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsmFlavor {
    FasmX86_64,
    Gas32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSymbol {
    pub name: String,
    pub hash: u64,
}

#[derive(Debug)]
pub enum SymtabGenError {
    Parse(String),
    SymbolName(String),
    HashMismatch {
        name: String,
        expected: u64,
        found: u64,
    },
    DuplicateHash {
        hash: u64,
        first: String,
        second: String,
    },
    NoRuntimeSymbols,
}

impl std::fmt::Display for SymtabGenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "object parse failed: {e}"),
            Self::SymbolName(e) => write!(f, "reading symbol name failed: {e}"),
            Self::HashMismatch {
                name,
                expected,
                found,
            } => write!(
                f,
                "runtime word hash mismatch for {name}: suffix {expected:#018x}, computed {found:#018x}"
            ),
            Self::DuplicateHash {
                hash,
                first,
                second,
            } => write!(
                f,
                "duplicate runtime symbol hash {hash:#018x}: {first} and {second}"
            ),
            Self::NoRuntimeSymbols => write!(f, "runtime object exported no runtime ABI symbols"),
        }
    }
}

impl std::error::Error for SymtabGenError {}

pub fn extract_runtime_symbols(obj: &[u8]) -> Result<Vec<RuntimeSymbol>, SymtabGenError> {
    let file = object::File::parse(obj).map_err(|e| SymtabGenError::Parse(e.to_string()))?;
    let mut by_hash: BTreeMap<u64, RuntimeSymbol> = BTreeMap::new();

    for sym in file.symbols() {
        if sym.is_undefined() || !sym.is_global() {
            continue;
        }
        let Ok(name) = sym.name() else {
            continue;
        };
        if !is_runtime_export(name) {
            continue;
        }
        let hash = runtime_symbol_hash(name)?;
        if let Some(existing) = by_hash.get(&hash) {
            if existing.name != name {
                return Err(SymtabGenError::DuplicateHash {
                    hash,
                    first: existing.name.clone(),
                    second: name.to_string(),
                });
            }
            continue;
        }
        by_hash.insert(
            hash,
            RuntimeSymbol {
                name: name.to_string(),
                hash,
            },
        );
    }

    if by_hash.is_empty() {
        return Err(SymtabGenError::NoRuntimeSymbols);
    }

    Ok(by_hash.into_values().collect())
}

pub fn is_runtime_export(name: &str) -> bool {
    is_runtime_word(name) || name.starts_with("__lang_") || name == "__stack_overflow"
}

pub fn runtime_symbol_hash(name: &str) -> Result<u64, SymtabGenError> {
    Ok(lmod::hash::linked_symbol_hash(name.as_bytes()))
}

fn is_runtime_word(name: &str) -> bool {
    let Some(hex) = name.strip_prefix("w_") else {
        return false;
    };
    hex.len() == 16 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn render_asm(symbols: &[RuntimeSymbol], flavor: AsmFlavor) -> String {
    match flavor {
        AsmFlavor::FasmX86_64 => render_fasm_x86_64(symbols),
        AsmFlavor::Gas32 => render_gas32(symbols),
    }
}

pub fn render_names(symbols: &[RuntimeSymbol]) -> String {
    let mut out = String::new();
    for sym in symbols {
        let _ = writeln!(&mut out, "{:016x} {}", sym.hash, sym.name);
    }
    out
}

fn render_fasm_x86_64(symbols: &[RuntimeSymbol]) -> String {
    let mut out = String::new();
    let _ = writeln!(&mut out, "format ELF64");
    for sym in symbols {
        let _ = writeln!(&mut out, "extrn {}", sym.name);
    }
    let _ = writeln!(&mut out, "section '.lang.symtab' writeable");
    let _ = writeln!(&mut out, "public __lang_symtab_start");
    let _ = writeln!(&mut out, "public __lang_symtab_end");
    let _ = writeln!(&mut out, "__lang_symtab_start:");
    let _ = writeln!(&mut out, "  dd {}", symbols.len());
    let _ = writeln!(&mut out, "  dd 0");
    for sym in symbols {
        let _ = writeln!(&mut out, "  dq 0x{:016x}", sym.hash);
        let _ = writeln!(&mut out, "  dq {}", sym.name);
    }
    let _ = writeln!(&mut out, "__lang_symtab_end:");
    out
}

fn render_gas32(symbols: &[RuntimeSymbol]) -> String {
    let mut out = String::new();
    let _ = writeln!(&mut out, ".section .lang.symtab, \"a\", %progbits");
    let _ = writeln!(&mut out, ".global __lang_symtab_start");
    let _ = writeln!(&mut out, ".global __lang_symtab_end");
    let _ = writeln!(&mut out, ".align 3");
    let _ = writeln!(&mut out, "__lang_symtab_start:");
    let _ = writeln!(&mut out, "    .word {}", symbols.len());
    let _ = writeln!(&mut out, "    .word 0");
    for sym in symbols {
        let lo = sym.hash as u32;
        let hi = (sym.hash >> 32) as u32;
        let _ = writeln!(&mut out, "    .word 0x{lo:08x}");
        let _ = writeln!(&mut out, "    .word 0x{hi:08x}");
        let _ = writeln!(&mut out, "    .word {}", sym.name);
        let _ = writeln!(&mut out, "    .word 0");
    }
    let _ = writeln!(&mut out, "__lang_symtab_end:");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_export_filter_matches_contract() {
        assert!(is_runtime_export("w_accb676a903a06d9"));
        assert!(is_runtime_export("__lang_ds_high"));
        assert!(is_runtime_export("__stack_overflow"));
        assert!(!is_runtime_export("__task_yield"));
        assert!(!is_runtime_export("__mmio_mem"));
        assert!(!is_runtime_export("w_not_hex"));
    }

    #[test]
    fn runtime_word_hash_uses_suffix() {
        assert_eq!(
            runtime_symbol_hash("w_accb676a903a06d9").unwrap(),
            0xaccb676a903a06d9
        );
        assert_ne!(
            runtime_symbol_hash("w_accb676a903a06d9").unwrap(),
            lmod::hash::fnv1a_u64(b"w_accb676a903a06d9")
        );
    }

    #[test]
    fn runtime_lang_symbol_hash_uses_name() {
        assert_eq!(
            runtime_symbol_hash("__lang_ds_high").unwrap(),
            lmod::hash::fnv1a_u64(b"__lang_ds_high")
        );
    }

    #[test]
    fn render_fasm_is_count_prefixed_and_sorted_input_preserved() {
        let symbols = vec![
            RuntimeSymbol {
                name: "__lang_ds_high".to_string(),
                hash: 0x11,
            },
            RuntimeSymbol {
                name: "w_accb676a903a06d9".to_string(),
                hash: 0xaccb676a903a06d9,
            },
        ];
        let asm = render_asm(&symbols, AsmFlavor::FasmX86_64);
        assert!(asm.contains("section '.lang.symtab'"));
        assert!(asm.contains("dd 2"));
        assert!(asm.contains("dq 0xaccb676a903a06d9"));
        assert!(asm.contains("dq w_accb676a903a06d9"));
    }

    #[test]
    fn render_gas32_uses_little_endian_words_for_hash_and_addr() {
        let symbols = vec![RuntimeSymbol {
            name: "w_accb676a903a06d9".to_string(),
            hash: 0xaccb676a903a06d9,
        }];
        let asm = render_asm(&symbols, AsmFlavor::Gas32);
        assert!(asm.contains(".word 0x903a06d9"));
        assert!(asm.contains(".word 0xaccb676a"));
        assert!(asm.contains(".word w_accb676a903a06d9"));
        assert!(asm.contains(".word 0"));
    }
}
